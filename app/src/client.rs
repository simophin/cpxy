use anyhow::{anyhow, bail, Context};
use futures_lite::io::split;
use futures_util::{select, FutureExt};
use smol::channel::{bounded, Sender, TrySendError};
use smol_timeout::TimeoutExt;
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::buf::{Buf, RWBuffer};
use crate::config::*;
use crate::counter::Counter;
use crate::fetch::send_http;
use crate::handshake::Handshaker;
use crate::io::{TcpListener, TcpStream, UdpSocket};
use crate::proxy::protocol::{ProxyRequest, ProxyResult};
use crate::proxy::request_proxy_upstream;
use crate::socks5::Address;
use crate::stream::AsyncReadWrite;
use crate::udp_relay;
use crate::utils::copy_duplex;
use futures_lite::future::race;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use smol::{spawn, Executor, Task, Timer};

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

struct UdpConnection {
    established: Instant,
    sender: Sender<Buf>,
    task: Task<anyhow::Result<()>>,
}

type UdpClient = BTreeMap<SocketAddr, UdpConnection>;

fn serve_udp_conn(
    socket: Arc<UdpSocket>,
    client_addr: SocketAddr,
    dst: SocketAddr,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> UdpConnection {
    let (sender, receiver) = bounded::<Buf>(10);
    let task = spawn(async move {
        let upstreams = config.find_best_upstream(&stats, &Address::IP(dst));
        if !upstreams.is_empty() {
            let proxy_req = ProxyRequest::UDPConn { dst };
            for (name, config) in upstreams {
                log::info!("Trying upstream {name} for UDP://{dst}");
                match request_proxy_upstream(config, &proxy_req).await {
                    Ok((ProxyResult::Granted { .. }, upstream, latency)) => {
                        log::info!(
                            "Upstream {name} granted for UDP://{dst}, latency = {latency:?}"
                        );

                        let (mut upstream_r, mut upstream_w) = split(upstream);

                        let _tcp_task: Task<anyhow::Result<()>> = spawn(async move {
                            let mut length_buf = [0u8; 2];
                            loop {
                                upstream_r
                                    .read_exact(&mut length_buf)
                                    .await
                                    .context("Reading message length")?;

                                let size = u16::from_be_bytes(length_buf) as usize;
                                let mut buf = Buf::new_with_len(size, size);
                                upstream_r
                                    .read_exact(&mut buf)
                                    .await
                                    .context("Reading message")?;
                                socket
                                    .send_to(&buf, &client_addr)
                                    .await
                                    .context("Sending back to UDP")?;
                            }
                        });

                        while let Ok(buf) = receiver.recv().await {
                            if buf.len() < 65536 {
                                bail!(
                                    "Unable to send UDP packet more than 65536: got {}",
                                    buf.len()
                                );
                            }

                            upstream_w
                                .write_all(buf.len().to_be_bytes().as_ref())
                                .await
                                .context("Writing UDP length")?;
                            upstream_w
                                .write_all(&buf)
                                .await
                                .context("Writing UDP data")?;
                        }

                        return Ok(());
                    }
                    Ok((r, _, _)) => {
                        log::info!("Upstream {name} rejected UDP://{dst} with result: {r:?}");
                        return Err(r.into());
                    }
                    Err(e) => {
                        log::error!("Error connecting to upstream for UDP://{dst}: {e:?}");
                        continue;
                    }
                }
            }
        }

        Ok(())
    });
    UdpConnection {
        sender,
        task,
        established: Instant::now(),
    }
}

async fn start_udp_client_with(
    socket: UdpSocket,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<Task<anyhow::Result<()>>> {
    socket.set_trasnparent()?;
    socket.set_receive_original_dst()?;
    let socket = Arc::new(socket);
    Ok(spawn(async move {
        let interval = Duration::from_secs(60);
        let mut clients = HashMap::<SocketAddr, UdpClient>::new();
        let mut buf = Buf::new_for_udp();
        let mut cleanup_timer = futures_util::FutureExt::fuse(Timer::at(Instant::now() + interval));

        loop {
            let (len, src, orig_dst) = select! {
                msg = socket.recvmsg(&mut buf).fuse() => {
                    match msg {
                        Ok((len, src, Some(dst))) => (len, src, dst),
                        Ok(_) => {
                            log::warn!("No ORIGDST specified in UDP message");
                            continue;
                        }
                        Err(e) => {
                            log::error!("Error receiving UDP message");
                            return Err(e);
                        }
                    }
                }
                _ = cleanup_timer => {
                    log::info!("Executing cleanup timer");
                    continue;
                }
            };
            log::info!("Received message from {src}, orig_dst = {orig_dst}, len = {len}");
            buf.set_len(len);

            match clients
                .entry(src)
                .or_insert_with(|| UdpClient::default())
                .entry(orig_dst)
                .or_insert_with(|| {
                    serve_udp_conn(socket.clone(), src, orig_dst, config.clone(), stats.clone())
                })
                .sender
                .try_send(buf)
            {
                Ok(_) => buf = Buf::new_for_udp(),
                Err(TrySendError::Full(b)) | Err(TrySendError::Closed(b)) => {
                    buf = b;
                    buf.set_len(buf.capacity());
                }
            };
        }
    }))
}

pub async fn run_tcp_client_with(
    proxy_listener: TcpListener,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    let executor = Executor::new();
    loop {
        let (sock, addr) = select! {
            v1 = proxy_listener.accept().fuse() => v1.context("Listening for SOCKS5/SOCKS4/HTTP/TPROXY connection")?,
            _ = executor.tick().fuse() => {
                let mut tick_num = 10;
                while executor.try_tick() && tick_num >= 0 {
                    tick_num -= 1;
                }
                continue;
            },
        };

        let config = config.clone();
        let stats = stats.clone();
        executor
            .spawn(async move {
                log::info!("Client {addr} connected");

                if let Err(e) = serve_proxy_client(sock, config, stats).await {
                    log::error!("Error serving client {addr}: {e:?}");
                }

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
    let mut current_tasks = Vec::<Task<_>>::with_capacity(2);

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
        current_tasks.clear();

        let proxy_listener = match TcpListener::bind(&Address::IP(config.socks5_address)).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error listening for TCP proxy: {e:?}");
                continue;
            }
        };

        let socket = match UdpSocket::bind_raw(&config.socks5_address).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error listening for UDP tproxy: {e:?}");
                continue;
            }
        };

        match start_udp_client_with(socket, config.clone(), stats.clone()).await {
            Ok(task) => current_tasks.push(task),
            Err(e) => {
                log::error!("Error starting UDP client: {e:?}");
                continue;
            }
        };

        {
            let config = config.clone();
            current_tasks.push(spawn(async move {
                run_tcp_client_with(proxy_listener, config, stats).await
            }));
        }

        log::info!("Proxy server listening on {}", config.socks5_address);
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
    mut socks: TcpStream,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::new(128, 65536);
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
                            Ok(upstream) => {
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

        ProxyRequest::UDP => match udp_relay::Relay::new(config, stats).await {
            Ok((r, a)) => {
                handshaker.respond_ok(&mut socks, Some(a)).await?;
                race(r.run(), drain_socks(socks)).await
            }
            Err(e) => {
                handshaker.respond_err(&mut socks).await?;
                Err(e.into())
            }
        },
        ProxyRequest::UDPConn { .. } | ProxyRequest::DNS { .. } => {
            handshaker.respond_err(&mut socks).await?;
            bail!("Unsupported DNS request")
        }
    }
}
