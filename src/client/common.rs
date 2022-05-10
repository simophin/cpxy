use std::time::Instant;

use anyhow::{anyhow, Context};
use futures::{AsyncRead, AsyncWrite};

use crate::{
    config::ClientConfig,
    handshake::Handshaker,
    protocol::{Protocol, Stats, TrafficType},
    socks5::Address,
    utils::copy_duplex,
};

use super::ClientStatistics;

pub async fn serve_stream_based_conn(
    dst: Address<'_>,
    initial_data: Option<&[u8]>,
    client_config: &ClientConfig,
    stats: &ClientStatistics,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let mut upstreams = client_config.find_best_upstream(TrafficType::Stream, stats, &dst);
    let mut last_error = None;

    while let Some((name, config)) = upstreams.pop() {
        log::debug!("Trying TCP:://{dst} on {name}");

        let protocol_stats = stats
            .upstreams
            .get(name)
            .map(|s| Stats {
                tx: s.tx.clone(),
                rx: s.rx.clone(),
            })
            .unwrap_or_default();

        let start = Instant::now();

        match config
            .protocol
            .new_stream(&dst, initial_data, &protocol_stats, client_config.fwmark)
            .await
            .with_context(|| format!("Requesting new streaming connection from {name}"))
        {
            Ok(upstream) => {
                let latency = start.elapsed();
                handshaker
                    .respond_ok(&mut stream, None)
                    .await
                    .context("Responding OK to handshaker")?;
                log::debug!("TCP:://{dst} connected on {name}");
                stats.update_upstream(name, latency);
                return copy_duplex(stream, upstream, None, None).await;
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
