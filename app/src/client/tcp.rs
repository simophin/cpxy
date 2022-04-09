use std::{net::SocketAddr, time::Duration};

use anyhow::{anyhow, bail, Context};
use futures_lite::{AsyncRead, AsyncWrite};
use smol_timeout::TimeoutExt;

use crate::{
    config::ClientConfig,
    fetch::send_http,
    handshake::Handshaker,
    io::TcpStream,
    proxy::{
        protocol::{ProxyRequest, ProxyResult},
        request_proxy_upstream,
    },
    socks5::Address,
    utils::copy_duplex,
};

use super::ClientStatistics;

pub async fn serve_tcp_proxy_conn(
    req: ProxyRequest,
    config: &ClientConfig,
    stats: &ClientStatistics,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let dst = match &req {
        ProxyRequest::HTTP { dst, .. } | ProxyRequest::TCP { dst } => dst,
        _ => panic!("Unable to handle non-tcp connection"),
    };

    let mut upstreams = config.find_best_upstream(stats, dst);

    if !upstreams.is_empty() {
        let mut last_error = None;
        while let Some((name, config)) = upstreams.pop() {
            log::info!("Trying upstream {name} for {dst}");
            match request_proxy_upstream(config, &req).await {
                Ok((ProxyResult::Granted { bound_address, .. }, upstream, latency)) => {
                    log::debug!(
                        "Upstream granted with address = {bound_address:?}, latency = {latency:?}"
                    );
                    handshaker.respond_ok(&mut stream, bound_address).await?;
                    stats.update_upstream(name, latency);
                    let (upstream_tx_bytes, upstream_rx_bytes) = match stats.upstreams.get(name) {
                        Some(stats) => (Some(stats.tx.clone()), Some(stats.rx.clone())),
                        None => (None, None),
                    };
                    return copy_duplex(upstream, stream, upstream_rx_bytes, upstream_tx_bytes)
                        .await
                        .context("Redirecting upstream traffic");
                }
                Ok((result, _, _)) => {
                    log::debug!("Upstream deined with result = {result:?}");
                    handshaker.respond_err(&mut stream).await?;
                    return Err(result.into());
                }
                Err(e) => {
                    log::debug!("Upstream error: {e}");
                    last_error = Some(e);
                }
            };
        }

        log::info!("No usable upstreams for {dst}, last_error = {last_error:?}");
        handshaker.respond_err(&mut stream).await?;
        Err(last_error.unwrap())
    } else if config.allow_direct(dst) {
        log::info!("Connecting directly to {dst}");
        match req {
            ProxyRequest::HTTP { dst, https, req } => match send_http(https, &dst, req).await {
                Ok(upstream) => {
                    handshaker.respond_ok(&mut stream, None).await?;
                    copy_duplex(upstream, stream, None, None).await
                }
                Err(e) => {
                    handshaker.respond_err(&mut stream).await?;
                    Err(e.into())
                }
            },
            ProxyRequest::TCP { dst } => match prepare_direct_tcp(dst).await {
                Ok((bound_address, upstream)) => {
                    handshaker.respond_ok(&mut stream, bound_address).await?;
                    copy_duplex(upstream, stream, None, None).await
                }
                Err(e) => {
                    handshaker.respond_err(&mut stream).await?;
                    Err(e.into())
                }
            },
            _ => bail!("Unknown proxy request {req:?}"),
        }
    } else {
        log::info!("Blocking connection to {dst}");
        handshaker.respond_err(&mut stream).await?;
        Ok(())
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
    Ok((stream.local_addr().ok(), stream))
}
