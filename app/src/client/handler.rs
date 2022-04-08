use std::sync::Arc;

use anyhow::{bail, Context};
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, Stream, StreamExt};
use smol::{spawn, Task};

use crate::{
    buf::RWBuffer,
    config::ClientConfig,
    fetch::send_http,
    handshake::Handshaker,
    io::{TcpListener, TcpStream},
    proxy::{
        protocol::{ProxyRequest, ProxyResult},
        request_proxy_upstream,
    },
    socks5::Address,
    utils::copy_duplex,
};

use super::ClientStatistics;

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

        // let socket = match UdpSocket::bind_raw(&config.socks5_address).await {
        //     Ok(v) => v,
        //     Err(e) => {
        //         log::error!("Error listening for UDP tproxy: {e:?}");
        //         continue;
        //     }
        // };

        // match start_udp_client_with(socket, config.clone(), stats.clone()).await {
        //     Ok(task) => current_tasks.push(task),
        //     Err(e) => {
        //         log::error!("Error starting UDP client: {e:?}");
        //         continue;
        //     }
        // };

        {
            let config = config.clone();
            current_tasks.push(spawn(async move {
                super::run_tcp_client_with(proxy_listener, config, stats).await
            }));
        }

        log::info!("Proxy server listening on {}", config.socks5_address);
    }
}

async fn drain_socks(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 24];
    while socks.read(buf.as_mut_slice()).await? > 0 {}
    Ok(())
}

pub async fn serve_proxy_client(
    mut socks: TcpStream,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::new(128, 65536);
    let transparent_addr = socks.get_original_dst();
    let (handshaker, req) = Handshaker::start(&mut socks, transparent_addr, &mut buf)
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
                    ProxyRequest::TCP { dst } => match super::prepare_direct_tcp(dst).await {
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

        ProxyRequest::UDP => {
            // match udp_relay::Relay::new(config, stats).await {
            //     Ok((r, a)) => {
            //         handshaker.respond_ok(&mut socks, Some(a)).await?;
            //         race(r.run(), drain_socks(socks)).await
            //     }
            //     Err(e) => {
            //         handshaker.respond_err(&mut socks).await?;
            //         Err(e.into())
            //     }
            // }
            todo!()
        }
        ProxyRequest::DNS { .. } => {
            handshaker.respond_err(&mut socks).await?;
            bail!("Unsupported DNS request")
        }
    }
}
