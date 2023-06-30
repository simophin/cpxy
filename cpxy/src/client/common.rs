use std::time::Instant;

use anyhow::{anyhow, Context};

use crate::config::ClientConfig;
use crate::protocol::{Protocol, ProtocolStream, ProxyRequest};

use super::ClientStatistics;

pub async fn find_and_connect_stream(
    req: &ProxyRequest,
    client_config: &ClientConfig,
    stats: &ClientStatistics,
) -> anyhow::Result<ProtocolStream> {
    let mut upstreams = client_config.find_best_upstream(stats, req)?;
    let mut last_error = None;

    while let Some((name, config)) = upstreams.pop() {
        log::debug!("Trying {req:?} on {name}");

        let protocol_stats = stats.get_protocol_stats(name).unwrap_or_default();
        let start = Instant::now();

        match config
            .protocol
            .new_stream(req, &protocol_stats, client_config.fwmark)
            .await
            .with_context(|| format!("Requesting new streaming connection from {name}"))
        {
            Ok(upstream) => {
                let latency = start.elapsed();
                stats.update_upstream(name, latency);
                return Ok(upstream);
            }
            Err(err) => {
                log::error!("Error connecting to upstream: {name}: {err:?}");
                last_error.replace(err)
            }
        };
    }

    Err(last_error.unwrap_or_else(|| anyhow!("No upstream available")))
}
