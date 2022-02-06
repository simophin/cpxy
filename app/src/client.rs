use anyhow::Context;
use bytes::Bytes;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use crate::config::*;
use crate::handshake::Handshaker;
use crate::io::{TcpListener, TcpStream};
use crate::proxy::protocol::{ProxyRequest, ProxyResult};
use crate::proxy::request_proxy_upstream;
use crate::udp_relay;
use crate::utils::{copy_duplex, RWBuffer};
use futures_lite::future::race;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use smol::{spawn, Task};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct UpstreamStatistics {
    pub tx: Arc<AtomicUsize>,
    pub rx: Arc<AtomicUsize>,
    pub last_activity: Arc<AtomicU64>,
}

#[derive(Default, Serialize, Deserialize, Debug)]
pub struct ClientStatistics {
    pub upstreams: HashMap<String, UpstreamStatistics>,
}

impl ClientStatistics {
    pub fn new(c: &ClientConfig) -> Self {
        Self {
            upstreams: c
                .upstreams
                .iter()
                .map(|(n, _)| (n.clone(), Default::default()))
                .collect(),
        }
    }

    pub fn update_upstream(&self, name: &str) {
        if let Some(stats) = self.upstreams.get(name) {
            stats.last_activity.store(UNIX_EPOCH.elapsed().unwrap().as_secs(), Ordering::Relaxed);
        }
    }
}

pub async fn run_client(
    mut config_stream: impl Stream<Item = (Arc<ClientConfig>, Arc<ClientStatistics>)>
        + Send
        + Sync
        + Unpin,
) -> anyhow::Result<()> {
    let mut current_task: Option<Task<anyhow::Result<()>>> = None;

    loop {
        let (config, stats) = match config_stream.next().await {
            Some(v) => v,
            None => {
                log::info!("Socks5 server stopped");
                return Ok(());
            }
        };

        log::debug!("Using configuration {config:?}");
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
        log::info!("Socks5 server listening on {}", config.socks5_address);

        let config = config.clone();
        let stats = stats.clone();
        current_task = Some(spawn(async move {
            loop {
                let (sock, addr) = match listener.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        log::error!("Error listening on {}", config.socks5_address);
                        return Err(e.into());
                    }
                };

                let config = config.clone();
                let stats = stats.clone();
                spawn(async move {
                    log::info!("Client {addr} connected");

                    if let Err(e) = serve_proxy_client(sock.is_v4(), sock, config, stats).await {
                        log::error!("Error serving client {addr}: {e}");
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
    let (handshaker, req) = Handshaker::start(&mut socks, &mut buf).await.context("Handshaking")?;
    log::info!("Requesting to proxy {req:?}");

    match &req {
        ProxyRequest::TCP { dst } | ProxyRequest::Http { dst, .. } => {
            let mut upstreams = config.find_best_upstream(stats.as_ref(), &dst);

            if !upstreams.is_empty() {
                let mut last_error = None;
                while let Some((name, config)) = upstreams.pop() {
                    log::info!("Trying upstream {name} for {dst}");
                    match request_proxy_upstream(&config, &req).await {
                        Ok((ProxyResult::Granted { bound_address }, upstream)) => {
                            handshaker.respond_ok(&mut socks, bound_address).await?;
                            stats.update_upstream(name);
                            let (upstream_tx_bytes, upstream_rx_bytes) = match stats.upstreams.get(name)
                            {
                                Some(stats) => (Some(stats.tx.clone()), Some(stats.rx.clone())),
                                None => (None, None),
                            };
                            return copy_duplex(upstream, socks, upstream_rx_bytes, upstream_tx_bytes).await
                        }
                        Ok((result, _)) => {
                            handshaker.respond_err(&mut socks).await?;
                            return Err(result.into())
                        }
                        Err(e) => {
                            log::debug!("Upstream error: {e}");
                            last_error = Some(e);
                        }
                    };
                }

                log::info!("No usable upstreams for {dst}, last_error = {last_error:?}");
                handshaker.respond_err(&mut socks).await?;
                Err(last_error.unwrap())
            } else {
                log::info!("Connecting directly to {dst}");
                match prepare_direct_tcp(&req).await {
                    Ok((bound_address, upstream)) => {
                        handshaker.respond_ok(&mut socks, bound_address).await?;
                        copy_duplex(upstream, socks, None, None).await
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
