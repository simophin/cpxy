use anyhow::{anyhow, Context};
use futures::{AsyncRead, AsyncWrite};

use crate::{
    config::ClientConfig, handshake::Handshaker, protocol::Protocol, proxy::protocol::ProxyRequest,
    socks5::Address, utils::copy_duplex,
};

use super::ClientStatistics;

pub async fn serve_stream_based_conn(
    dst: Address<'_>,
    proxy_request: &ProxyRequest<'_>,
    config: &ClientConfig,
    stats: &ClientStatistics,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let mut upstreams = config.find_best_upstream(&proxy_request, stats, &dst);
    let mut last_error = None;

    while let Some((name, config)) = upstreams.pop() {
        log::debug!("Trying {proxy_request:?} on {name}");

        match config
            .protocol
            .new_stream_conn(&proxy_request)
            .await
            .with_context(|| format!("Requesting new streaming connection from {name}"))
        {
            Ok((upstream, latency)) => {
                handshaker
                    .respond_ok(&mut stream, None)
                    .await
                    .context("Responding OK to handshaker")?;
                log::debug!("{proxy_request:?} connected on {name}");
                let (tx, rx) = match stats.upstreams.get(name) {
                    Some(s) => (Some(s.tx.clone()), Some(s.rx.clone())),
                    None => (None, None),
                };

                stats.update_upstream(name, latency);
                return copy_duplex(stream, upstream, tx, rx).await;
            }
            Err(err) => {
                log::error!("Error connecting to upstream: {name}: {err:?}");
                last_error.replace(err)
            }
        };
    }

    handshaker
        .respond_err(&mut stream)
        .await
        .context("Responding Err to handshaker")?;
    Err(last_error.unwrap_or_else(|| anyhow!("No upstreams available")))
}
