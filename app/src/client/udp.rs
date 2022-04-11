use anyhow::bail;
use futures_lite::{future::race, AsyncRead, AsyncReadExt, AsyncWrite};
use smol::channel::{Receiver, Sender};

use crate::{
    buf::Buf,
    config::ClientConfig,
    handshake::Handshaker,
    proxy::{
        protocol::{ProxyRequest, ProxyResult},
        request_proxy_upstream,
    },
    socks5::UdpPacket,
    udp_relay::new_udp_relay,
};

use super::{utils::request_best_upstream, ClientStatistics, UpstreamStatistics};

struct UdpPacketPayloadRef;

pub async fn serve_udp_proxy_conn(
    c: &ClientConfig,
    stats: &ClientStatistics,
    is_v4: bool,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let (relay_addr, tx, rx) = match new_udp_relay(is_v4).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("Error creating UDP relay: {e:?}");
            handshaker.respond_err(&mut stream).await?;
            return Err(e);
        }
    };

    handshaker.respond_ok(&mut stream, Some(relay_addr)).await?;

    // Wait for first packet to decide where to go
    let pkt = rx.recv().await?;
    let addr = pkt.addr().into_owned();

    match request_best_upstream(c, stats, &addr, &ProxyRequest::UDP { initial_data: pkt }).await {
        Ok((_, upstream, stat)) => {
            return race(
                copy_between_relay_and_stream(tx, rx, upstream, stat),
                drain_socks(stream),
            )
            .await;
        }
        Err(e) => {
            log::warn!("Error requesting best upstream for UDP://{addr}: {e:?}");
        }
    };

    if c.allow_direct(&addr) {
        return serve_udp_relay_directly(tx, rx).await;
    }

    bail!("There's no where for UDP packet to go")
}

async fn serve_udp_relay_directly(
    tx: Sender<UdpPacket<Buf>>,
    rx: Receiver<UdpPacket<Buf>>,
) -> anyhow::Result<()> {
    todo!()
}

async fn copy_between_relay_and_stream(
    tx: Sender<UdpPacket<Buf>>,
    rx: Receiver<UdpPacket<Buf>>,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    stats: Option<&UpstreamStatistics>,
) -> anyhow::Result<()> {
    Ok(())
}

async fn drain_socks(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 24];
    while socks.read(buf.as_mut_slice()).await? > 0 {}
    Ok(())
}
