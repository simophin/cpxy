use std::{net::SocketAddr, sync::Arc};

use super::req::Message;
use anyhow::{anyhow, Context};
use futures_lite::Future;
use futures_util::{select, FutureExt};
use smol::spawn;

use crate::{
    buf::Buf,
    io::{TcpListener, TcpStream, UdpSocket},
    socks5::Address,
};

use super::DnsResultCache;

pub async fn serve<T>(
    m: SocketAddr,
    upstream: impl Fn(Buf) -> T + Send + Sync + Clone + 'static,
    cache: Arc<DnsResultCache>,
) -> anyhow::Result<()>
where
    T: Future<Output = anyhow::Result<Buf>> + Send + Sync,
{
    let udp_server = UdpSocket::bind_raw(&m)
        .await
        .context("Binding DNS UDP server")?;

    let tcp_server = TcpListener::bind(&Address::IP(m))
        .await
        .context("Binding DNS TCP server")?;

    let mut msg_buf = Buf::new_with_len(65536, 65536);

    loop {
        select! {
            v = udp_server.recv_from(&mut msg_buf).fuse() => {
                let (len, addr) = v?;
                msg_buf.set_len(len);
                if let Err(e) = handle_udp_packet(msg_buf, &addr, &upstream, &cache).await {
                    log::error!("Error serving UDP client: {addr}: {e:?}");
                }

                msg_buf = Buf::new_with_len(65536, 65536);
            }

            c = tcp_server.accept().fuse() => {
                let (c, addr) = c?;
                let upstream = upstream.clone();
                spawn(async move {
                    if let Err(e) = serve_tcp_client(c, &addr).await {
                        log::error!("Error serving TCP client: {addr}: {e:?}")
                    }
                }).detach();
            }
        }
    }
}

async fn serve_tcp_client(mut c: TcpStream, addr: &SocketAddr) -> anyhow::Result<()> {
    todo!()
}

async fn handle_udp_packet<T>(
    mut buf: Buf,
    addr: &SocketAddr,
    upstream: &(impl Fn(Buf) -> T + Send + Sync + Clone),
    cache: &Arc<DnsResultCache>,
) -> anyhow::Result<()>
where
    T: Future<Output = anyhow::Result<Buf>> + Send + Sync,
{
    let msg = Message::parse(&buf).ok_or_else(|| anyhow!("Error parsing DNS message"))?;
    log::debug!("Received DNS message: {msg:?} from {addr}");

    //TODO: Look for cache
    drop(msg);

    // Go to upstream
    let response = upstream(buf).await?;
    let response_msg =
        Message::parse(&response).ok_or_else(|| anyhow!("Error parsing response DNS message"))?;
    log::debug!("Received DNS reply: {response_msg:?}");

    todo!();
}
