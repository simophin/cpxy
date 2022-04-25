use anyhow::Context;
use futures::{AsyncRead, AsyncWrite};

use crate::{
    config::ClientConfig, handshake::Handshaker, proxy::protocol::ProxyRequest, socks5::Address,
};

use super::ClientStatistics;

pub async fn serve_tcp_proxy_conn(
    dst: Address<'_>,
    config: &ClientConfig,
    stats: &ClientStatistics,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let proxy_request = ProxyRequest::TCP { dst: dst.clone() };
    super::common::serve_stream_based_conn(
        dst.clone(),
        &proxy_request,
        config,
        stats,
        stream,
        handshaker,
    )
    .await
    .with_context(|| format!("Proxying TCP connection to {dst}"))
}
