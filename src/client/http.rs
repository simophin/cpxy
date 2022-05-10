use anyhow::Context;
use futures::{AsyncRead, AsyncWrite};

use crate::{config::ClientConfig, handshake::Handshaker, http::HttpRequest, socks5::Address};

use super::ClientStatistics;

pub async fn serve_http_proxy_conn(
    dst: Address<'_>,
    https: bool,
    req: HttpRequest<'_>,
    config: &ClientConfig,
    stats: &ClientStatistics,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    super::common::serve_stream_based_conn(
        dst.clone(),
        Some(&req.to_builder().finalise()),
        config,
        stats,
        stream,
        handshaker,
    )
    .await
    .with_context(|| format!("Proxying HTTP dst = {dst}"))
}
