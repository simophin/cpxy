use std::net::SocketAddr;

use anyhow::bail;
use futures_lite::{AsyncRead, AsyncWrite};

use crate::{
    config::ClientConfig,
    proxy::{
        protocol::{ProxyRequest, ProxyResult},
        request_proxy_upstream_with_config,
    },
    socks5::Address,
};

use super::{ClientStatistics, UpstreamStatistics};

pub async fn request_best_upstream<'a>(
    c: &ClientConfig,
    stats: &'a ClientStatistics,
    dst: &Address<'_>,
    req: &ProxyRequest<'_>,
) -> anyhow::Result<(
    Option<SocketAddr>,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    Option<&'a UpstreamStatistics>,
)> {
    let mut upstreams = c.find_best_upstream(stats, dst);

    if !upstreams.is_empty() {
        let mut last_error = None;
        while let Some((name, config)) = upstreams.pop() {
            log::info!("Trying upstream {name} for {dst}");
            match request_proxy_upstream_with_config(c.fwmark, config, req).await {
                Ok((ProxyResult::Granted { bound_address, .. }, upstream, latency)) => {
                    log::debug!(
                        "Upstream granted with address = {bound_address:?}, latency = {latency:?}"
                    );
                    stats.update_upstream(name, latency);
                    return Ok((bound_address, upstream, stats.upstreams.get(name)));
                }
                Ok((result, _, _)) => {
                    log::debug!("Upstream deined with result = {result:?}");
                    return Err(result.into());
                }
                Err(e) => {
                    log::debug!("Upstream error: {e}");
                    last_error = Some(e);
                }
            };
        }

        log::info!("No usable upstreams for {dst}, last_error = {last_error:?}");
        Err(last_error.unwrap())
    } else {
        bail!("No upstream available")
    }
}
