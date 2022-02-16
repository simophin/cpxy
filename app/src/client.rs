use anyhow::{anyhow, bail, Context};
use futures_util::{select, FutureExt};
use smol_timeout::TimeoutExt;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use crate::config::*;
use crate::counter::Counter;
use crate::dns::DnsResultCache;
use crate::fetch::send_http;
use crate::handshake::Handshaker;
use crate::io::{TcpListener, TcpStream};
use crate::proxy::protocol::{ProxyRequest, ProxyResult};
use crate::proxy::request_proxy_upstream;
use crate::socks5::Address;
use crate::stream::AsyncReadWrite;
use crate::transparent::serve_transparent_proxy_client;
use crate::udp_relay;
use crate::utils::{copy_duplex, RWBuffer};
use futures_lite::future::race;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use smol::{spawn, Task};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct UpstreamStatistics {
    pub tx: Arc<Counter>,
    pub rx: Arc<Counter>,
    pub last_activity: Arc<Counter>,
    pub last_latency: Arc<Counter>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
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

    pub fn update_upstream(&self, name: &str, latency: Duration) {
        if let Some(stats) = self.upstreams.get(name) {
            stats
                .last_activity
                .set(UNIX_EPOCH.elapsed().unwrap().as_secs() as usize);
            stats.last_latency.set(latency.as_millis() as usize);
        }
    }
}

pub async fn run_client_with(
    proxy_listener: TcpListener,
    transparent_listener: Option<TcpListener>,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
    mut shutdown_rx: impl Stream<Item = ()> + Clone + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    let _transparent_task = {
        match transparent_listener {
            Some(listener) => {
                let config = config.clone();
                let stats = stats.clone();

                Some(spawn(async move {
                    if let Err(e) = serve_transparent_proxy_client(
                        listener,
                        config,
                        stats,
                        Arc::new(DnsResultCache {}),
                    )
                    .await
                    {
                        log::error!("Error serving transparent proxy: {e:?}");
                    }
                }))
            }
            None => None,
        }
    };

    loop {
        let (sock, addr) = select! {
            v1 = proxy_listener.accept().fuse() => v1.context("Listening for SOCKS5 connection")?,
            _ = shutdown_rx.next().fuse() => {
                log::info!("Socks5 listening cancelled");
                return Ok(());
            }
        };

        let config = config.clone();
        let stats = stats.clone();
        let mut shutdown_rx = shutdown_rx.clone();
        spawn(async move {
            log::info!("Client {addr} connected");

            select! {
                result = serve_proxy_client(sock.is_v4(), sock, config, stats).fuse() => {
                    if let Err(e) = result {
                        log::error!("Error serving client {addr}: {e:?}");
                    }
                },
                _ = shutdown_rx.next().fuse() => {
                    log::info!("Cancelling service of SOCKS5 client {addr}");
                }
            };

            log::info!("Client {addr} disconnected");
        })
        .detach();
    }
}

pub async fn run_client(
    mut config_stream: impl Stream<Item = (Arc<ClientConfig>, Arc<ClientStatistics>)>
        + Send
        + Sync
        + Unpin,
) -> anyhow::Result<()> {
    let mut current_task: Option<Task<anyhow::Result<()>>> = None;
    let (mut shutdown_tx, shutdown_rx) = async_broadcast::broadcast::<()>(1);
    shutdown_tx.set_overflow(true);

    loop {
        log::debug!("Listening for next config");
        let (config, stats) = match config_stream.next().await {
            Some(v) => v,
            None => {
                log::info!("Socks5 server stopped");
                return Ok(());
            }
        };

        log::debug!("Using configuration {config:?}");
        if let Some(task) = current_task {
            let _ = shutdown_tx.broadcast(()).await;
            let _ = task.await;
        }

        let proxy_listener = match TcpListener::bind(&config.socks5_address).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error listening for socks proxy: {e:?}");
                current_task = None;
                continue;
            }
        };
        log::info!("Socks5 server listening on {}", config.socks5_address);

        #[cfg(target_os = "linux")]
        let transparent_listener = match &config.transparent_address {
            Some(addr) => match TcpListener::bind(addr).await {
                Ok(v) => Some(v),
                Err(e) => {
                    log::error!("Error listening for transparent proxy: {e:?}");
                    current_task = None;
                    continue;
                }
            },
            None => None,
        };

        #[cfg(not(target_os = "linux"))]
        let transparent_listener = None;

        let config = config.clone();
        let stats = stats.clone();
        let shutdown_rx = shutdown_rx.clone();
        current_task = Some(spawn(async move {
            run_client_with(
                proxy_listener,
                transparent_listener,
                config,
                stats,
                shutdown_rx,
            )
            .await
        }));
    }
}

async fn prepare_direct_tcp(
    dst: Address<'_>,
) -> anyhow::Result<(
    Option<SocketAddr>,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
)> {
    let stream = TcpStream::connect(&dst)
        .timeout(Duration::from_secs(2))
        .await
        .ok_or_else(|| anyhow!("Timeout connecting to {dst}"))??;
    Ok((stream.local_addr().ok(), AsyncReadWrite::new(stream)))
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
    let (handshaker, req) = Handshaker::start(&mut socks, &mut buf)
        .await
        .context("Handshaking")?;
    log::info!("Requesting to proxy {req:?}");

    match &req {
        ProxyRequest::TCP { dst } | ProxyRequest::HTTP { dst, .. } => {
            let mut upstreams = config.find_best_upstream(stats.as_ref(), &dst);

            if !upstreams.is_empty() {
                let mut last_error = None;
                while let Some((name, config)) = upstreams.pop() {
                    log::info!("Trying upstream {name} for {dst}");
                    match request_proxy_upstream(&config, &req).await {
                        Ok((ProxyResult::Granted { bound_address, .. }, upstream, latency)) => {
                            log::debug!("Upstream granted with address = {bound_address:?}, latency = {latency:?}");
                            handshaker.respond_ok(&mut socks, bound_address).await?;
                            stats.update_upstream(name, latency);
                            let (upstream_tx_bytes, upstream_rx_bytes) =
                                match stats.upstreams.get(name) {
                                    Some(stats) => (Some(stats.tx.clone()), Some(stats.rx.clone())),
                                    None => (None, None),
                                };
                            return copy_duplex(
                                upstream,
                                socks,
                                upstream_rx_bytes,
                                upstream_tx_bytes,
                            )
                            .await
                            .context("Redirecting upstream traffic");
                        }
                        Ok((result, _, _)) => {
                            log::debug!("Upstream deined with result = {result:?}");
                            handshaker.respond_err(&mut socks).await?;
                            return Err(result.into());
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
            } else if config.allow_direct(dst) {
                log::info!("Connecting directly to {dst}");
                match req {
                    ProxyRequest::HTTP { dst, https, req } => {
                        match send_http(https, &dst, req).await {
                            Ok((upstream, _)) => {
                                handshaker.respond_ok(&mut socks, None).await?;
                                copy_duplex(upstream, socks, None, None).await
                            }
                            Err(e) => {
                                handshaker.respond_err(&mut socks).await?;
                                Err(e.into())
                            }
                        }
                    }
                    ProxyRequest::TCP { dst } => match prepare_direct_tcp(dst).await {
                        Ok((bound_address, upstream)) => {
                            handshaker.respond_ok(&mut socks, bound_address).await?;
                            copy_duplex(upstream, socks, None, None).await
                        }
                        Err(e) => {
                            handshaker.respond_err(&mut socks).await?;
                            Err(e.into())
                        }
                    },
                    _ => bail!("Unknown proxy request {req:?}"),
                }
            } else {
                log::info!("Blocking connection to {dst}");
                handshaker.respond_err(&mut socks).await?;
                Ok(())
            }
        }

        ProxyRequest::UDP => match udp_relay::Relay::new(config, stats, is_v4).await {
            Ok((r, a)) => {
                handshaker.respond_ok(&mut socks, Some(a)).await?;
                race(r.run(), drain_socks(socks)).await
            }
            Err(e) => {
                handshaker.respond_err(&mut socks).await?;
                Err(e.into())
            }
        },
        ProxyRequest::DNS { .. } => {
            handshaker.respond_err(&mut socks).await?;
            bail!("Unsupported DNS request")
        }
    }
}
