use anyhow::anyhow;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::cipher::strategy::EncryptionStrategy;
use crate::handshake::Handshaker;
use crate::proxy::handler::{ProxyRequest, ProxyResult};
use crate::proxy::udp::{copy_socks5_udp_to_stream, copy_stream_to_socks5_udp};
use crate::utils::{copy_duplex, RWBuffer};
use futures_lite::future::race;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncWrite};
use smol::net::{TcpListener, TcpStream, UdpSocket};
use smol::{spawn, Task};
use smol_timeout::TimeoutExt;

pub async fn run_client(
    listener: TcpListener,
    upstream_host: &str,
    upstream_port: u16,
    socks5_udp_host: &str,
) -> anyhow::Result<()> {
    let upstream = format!("{upstream_host}:{upstream_port}");

    let clients: Arc<Mutex<HashMap<SocketAddr, Task<anyhow::Result<()>>>>> = Default::default();

    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!("Accepted client from: {addr}");

        let upstream = upstream.clone();

        if let Ok(mut m) = clients.lock() {
            let task: Task<anyhow::Result<()>> = {
                let addr = addr.clone();
                let clients = clients.clone();
                let socks5_udp_host = socks5_udp_host.to_string();
                spawn(async move {
                    if let Err(e) = serve_proxy_client(sock, upstream, socks5_udp_host).await {
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

async fn run_udp_proxy_relay(
    handshaker: Handshaker,
    proxy_result: ProxyResult,
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    upstream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    relay_host: String,
) -> anyhow::Result<()> {
    log::debug!("Accepted UDP proxy with result = {proxy_result}");
    match &proxy_result {
        ProxyResult::Granted { .. } => {}
        _ => {
            let err = anyhow!("Received error from proxy: {proxy_result:?}");
            handshaker.respond(&mut socks, proxy_result).await?;
            return Err(err);
        }
    };

    let socket = match UdpSocket::bind(format!("{relay_host}:0"))
        .await
        .map_err(|e| anyhow::Error::from(e))
        .and_then(|socket| Ok((socket.local_addr()?, socket)))
    {
        Ok((a, s)) => {
            log::debug!("Socks5-Relay-Udp listening on {a}");
            handshaker
                .respond(&mut socks, ProxyResult::Granted { bound_address: a })
                .await?;
            Arc::new(s)
        }
        Err(e) => {
            handshaker
                .respond(&mut socks, ProxyResult::ErrGeneric { msg: e.to_string() })
                .await?;
            return Err(e);
        }
    };

    let (r, w) = split(upstream);
    let last_addr: Arc<RwLock<Option<SocketAddr>>> = Default::default();
    let task1 = {
        let socket = socket.clone();
        let last_addr = last_addr.clone();
        spawn(async move { copy_socks5_udp_to_stream(&socket, w, last_addr).await })
    };

    let task2 = {
        let socket = socket.clone();
        let last_addr = last_addr.clone();
        spawn(async move { copy_stream_to_socks5_udp(r, &socket, last_addr).await })
    };

    race(task1, task2).await
}

async fn serve_proxy_client(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    upstream_addr: String,
    socks5_udp_host: String,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::default();
    let (handshaker, req) = Handshaker::start(&mut socks, &mut buf).await?;
    log::info!("Requesting to proxy {req:?}");

    let send_enc = EncryptionStrategy::pick_send(&req);
    let receive_enc = EncryptionStrategy::pick_receive(&req);

    let r = super::proxy::handler::request_proxy(&req, move |buf| async move {
        let upstream = TcpStream::connect(upstream_addr.as_str())
            .timeout(Duration::from_secs(2))
            .await
            .ok_or_else(|| anyhow!("Timeout"))??;
        super::cipher::client::connect(upstream, upstream_addr.as_str(), send_enc, receive_enc, buf)
            .await
    })
    .await;

    let upstream = match r {
        Ok((proxy_r, upstream)) => match req {
            ProxyRequest::SocksTCP(_) | ProxyRequest::Http(_) => {
                handshaker.respond(&mut socks, proxy_r).await?;
                upstream
            }
            ProxyRequest::SocksUDP => {
                return run_udp_proxy_relay(handshaker, proxy_r, socks, upstream, socks5_udp_host)
                    .await;
            }
        },
        Err(err) => {
            handshaker
                .respond(
                    &mut socks,
                    ProxyResult::ErrGeneric {
                        msg: err.to_string(),
                    },
                )
                .await?;
            return Err(err);
        }
    };

    copy_duplex(upstream, socks).await
}
