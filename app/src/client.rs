use anyhow::anyhow;
use bytes::Bytes;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::Arc;

use crate::config::*;
use crate::handshake::Handshaker;
use crate::io::{TcpListener, TcpStream};
use crate::proxy::protocol::{ProxyRequest, ProxyResult};
use crate::proxy::request_proxy_upstream;
use crate::udp_relay;
use crate::utils::{copy_duplex, RWBuffer};
use futures_lite::future::race;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream, StreamExt};
use futures_util::{select, FutureExt};
use serde::{Deserialize, Serialize};
use smol::{spawn, Task};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct UpstreamStatistics {
    pub tx: AtomicUsize,
    pub rx: AtomicUsize,
    pub last_activity: AtomicU64,
}

#[derive(Default, Serialize, Deserialize, Debug)]
pub struct ClientStatistics {
    pub upstreams: HashMap<String, UpstreamStatistics>,
}

pub async fn run_client(
    mut config_stream: impl Stream<Item = (Arc<ClientConfig>, Arc<ClientStatistics>)>
        + Send
        + Sync
        + Unpin
        + 'static,
) -> anyhow::Result<()> {
    let (shutdown_tx, shutdown_rx) = async_broadcast::broadcast::<()>(1);
    let mut current_task: Option<Task<anyhow::Result<()>>> = None;

    loop {
        let (config, stats) = config_stream
            .next()
            .await
            .ok_or_else(|| anyhow!("No initial config found"))?;

        log::debug!("Using configuration {config:?}");
        shutdown_tx.broadcast(()).await?;
        if let Some(task) = current_task {
            let _ = task.cancel().await;
        }

        let listener = match TcpListener::bind(&config.socks5_address).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error listening for socks proxy: {e}");
                current_task = None;
                continue;
            }
        };

        let mut shutdown_rx = shutdown_rx.clone();
        let config = config.clone();
        let stats = stats.clone();
        current_task = Some(spawn(async move {
            loop {
                let (sock, addr) = select! {
                    v = listener.accept().fuse() => v?,
                    _ = shutdown_rx.next().fuse() => return Ok(()),
                };

                let mut shutdown_rx = shutdown_rx.clone();
                let config = config.clone();
                let stats = stats.clone();
                spawn(async move {
                    log::info!("Client {addr} connected");

                    select! {
                        v = serve_proxy_client(sock.is_v4(), sock, config, stats).fuse() => {
                            if let Err(e) = v {
                                log::error!("Error serving client {addr}: {e}");
                            }
                        },
                        _ = shutdown_rx.next().fuse() => {},
                    }

                    log::info!("Client {addr} disconnected");
                })
                .detach();
            }
        }));
    }
}

async fn prepare_direct_tcp(
    req: &ProxyRequest,
) -> anyhow::Result<(
    SocketAddr,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
)> {
    let (dst, init_req) = match req {
        ProxyRequest::TCP { dst } => (dst, Bytes::new()),
        ProxyRequest::Http { dst, request } => (dst, request.clone()),
        ProxyRequest::UDP => unreachable!("Unexpected request type for direct tcp: {req:}"),
    };

    let mut stream = TcpStream::connect(&dst).await?;
    if !init_req.is_empty() {
        stream.write_all(init_req.as_ref()).await?;
    }
    Ok((stream.local_addr()?, stream))
}

async fn drain_socks(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 24];
    while socks.read(buf.as_mut_slice()).await? > 0 {}
    Ok(())
}

async fn serve_proxy_client(
    is_v4: bool,
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::default();
    let (handshaker, req) = Handshaker::start(&mut socks, &mut buf).await?;
    log::info!("Requesting to proxy {req:?}");

    match &req {
        ProxyRequest::TCP { dst } | ProxyRequest::Http { dst, .. } => {
            if let Some((name, config)) = config.find_best_upstream(stats, &dst) {
                log::info!("Using upstream {name} for {dst}");
                match request_proxy_upstream(&config, &req).await {
                    Ok((ProxyResult::Granted { bound_address }, upstream)) => {
                        handshaker.respond_ok(&mut socks, bound_address).await?;
                        copy_duplex(upstream, socks).await
                    }
                    Ok((result, _)) => {
                        handshaker.respond_err(&mut socks).await?;
                        Err(result.into())
                    }
                    Err(e) => {
                        handshaker.respond_err(&mut socks).await?;
                        Err(e.into())
                    }
                }
            } else {
                log::info!("Connecting directly to {dst}");
                match prepare_direct_tcp(&req).await {
                    Ok((bound_address, upstream)) => {
                        handshaker.respond_ok(&mut socks, bound_address).await?;
                        copy_duplex(upstream, socks).await
                    }
                    Err(e) => {
                        handshaker.respond_err(&mut socks).await?;
                        Err(e.into())
                    }
                }
            }
        }

        ProxyRequest::UDP => match udp_relay::Relay::new(config, stats, is_v4).await {
            Ok((r, a)) => {
                handshaker.respond_ok(&mut socks, a).await?;
                race(r.run(), drain_socks(socks)).await
            }
            Err(e) => {
                handshaker.respond_err(&mut socks).await?;
                Err(e.into())
            }
        },
    }
}
