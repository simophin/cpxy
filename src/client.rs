use anyhow::anyhow;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cipher::strategy::EncryptionStrategy;
use crate::handshake::Handshaker;
use crate::io::{TcpStream, UdpSocket};
use crate::proxy::protocol::{ProxyRequest, ProxyResult};
use crate::socks5::{serve_socks5_udp_stream_relay, Address};
use crate::utils::{copy_duplex, read_bincode_lengthed_async, write_bincode_lengthed, RWBuffer};
use futures_lite::future::race;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite};
use smol::net::TcpListener;
use smol::{spawn, Task};
use smol_timeout::TimeoutExt;

pub struct ClientConfig {
    pub upstream: Address,
    pub upstream_timeout: Duration,
    pub socks5_udp_host: String,
}

pub async fn run_client(listener: TcpListener, config: Arc<ClientConfig>) -> anyhow::Result<()> {
    let clients: Arc<Mutex<HashMap<SocketAddr, Task<anyhow::Result<()>>>>> = Default::default();

    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!("Accepted client from: {addr}");

        if let Ok(mut m) = clients.lock() {
            let task: Task<anyhow::Result<()>> = {
                let addr = addr.clone();
                let clients = clients.clone();
                let config = config.clone();
                spawn(async move {
                    if let Err(e) = serve_proxy_client(sock, config).await {
                        log::error!("Error serving client {addr}: {e}");
                    }
                    log::info!("Client {addr} disconnected");
                    if let Ok(mut m) = clients.lock() {
                        m.remove(&addr);
                    }

                    Ok(())
                })
            };
            m.insert(addr, task);
        }
    }
}

async fn prepare_upstream(
    c: &ClientConfig,
    req: &ProxyRequest,
) -> anyhow::Result<(
    ProxyResult,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
)> {
    let send_enc = EncryptionStrategy::pick_send(&req);
    let receive_enc = EncryptionStrategy::pick_receive(&req);

    log::debug!("EncryptionStrategy(send = {send_enc:?}, receive = {receive_enc})");

    let mut header = Vec::new();
    write_bincode_lengthed(&mut header, req)?;

    let upstream = TcpStream::connect(&c.upstream)
        .timeout(c.upstream_timeout)
        .await
        .ok_or_else(|| anyhow!("Timeout connecting to {}", c.upstream))??;

    let mut upstream = super::cipher::client::connect(
        upstream,
        c.upstream.get_host().as_ref(),
        send_enc,
        receive_enc,
        header,
    )
    .await?;

    Ok((read_bincode_lengthed_async(&mut upstream).await?, upstream))
}

async fn prepare_relay_udp(is_v4: bool) -> anyhow::Result<(UdpSocket, SocketAddr)> {
    let socks = UdpSocket::bind(is_v4).await?;
    let addr = socks.local_addr()?;
    Ok((socks, addr))
}

async fn drain_socks(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 24];
    while socks.read(buf.as_mut_slice()).await? > 0 {}
    Ok(())
}

async fn serve_proxy_client(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    config: Arc<ClientConfig>,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::default();
    let (handshaker, req) = Handshaker::start(&mut socks, &mut buf).await?;
    log::info!("Requesting to proxy {req:?}");

    match (prepare_upstream(config.as_ref(), &req).await, &req) {
        (
            Ok((ProxyResult::Granted { bound_address }, upstream)),
            ProxyRequest::TCP { .. } | ProxyRequest::Http { .. },
        ) => {
            log::info!("Proxying {req:?}");
            handshaker.respond_ok(&mut socks, bound_address).await?;
            copy_duplex(upstream, socks).await
        }
        (Ok((ProxyResult::Granted { .. }, upstream)), ProxyRequest::UDP { .. }) => {
            log::info!("Proxying {req:?}");
            let socket = match prepare_relay_udp(true).await {
                Ok((v, addr)) => {
                    handshaker.respond_ok(&mut socks, addr).await?;
                    v
                }
                Err(e) => {
                    handshaker.respond_err(&mut socks).await?;
                    return Err(e.into());
                }
            };
            race(
                drain_socks(socks),
                serve_socks5_udp_stream_relay(socket, upstream),
            )
            .await
        }
        (Ok((v, _)), _) => {
            handshaker.respond_err(&mut socks).await?;
            return Err(v.into());
        }
        (Err(e), _) => {
            handshaker.respond_err(&mut socks).await?;
            return Err(e.into());
        }
    }
}
