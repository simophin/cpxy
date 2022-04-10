use std::{net::SocketAddr, time::Duration};

use anyhow::{anyhow, bail, Context};
use futures_lite::{AsyncRead, AsyncWrite};
use smol_timeout::TimeoutExt;

use crate::{
    config::ClientConfig, handshake::Handshaker, io::TcpStream, proxy::protocol::ProxyRequest,
    socks5::Address, utils::copy_duplex,
};

use super::{utils::request_best_upstream, ClientStatistics};

pub async fn serve_tcp_proxy_conn(
    dst: Address<'static>,
    config: &ClientConfig,
    stats: &ClientStatistics,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let proxy_request = ProxyRequest::TCP { dst: dst.clone() };
    if let Ok((bound, upstream, stats)) =
        request_best_upstream(config, stats, &dst, &proxy_request).await
    {
        handshaker.respond_ok(&mut stream, bound).await?;
        copy_duplex(
            upstream,
            stream,
            stats.map(|s| s.rx.clone()),
            stats.map(|s| s.tx.clone()),
        )
        .await
        .context("Redirecting upstream traffic")
    } else if config.allow_direct(&dst) {
        match prepare_direct_tcp(&dst).await {
            Ok((bound, upstream)) => {
                handshaker.respond_ok(&mut stream, bound).await?;
                copy_duplex(upstream, stream, None, None).await
            }

            Err(err) => {
                handshaker.respond_err(&mut stream).await?;
                return Err(err);
            }
        }
    } else {
        handshaker.respond_err(&mut stream).await?;
        bail!("No where to direct TCP traffic");
    }
}

async fn prepare_direct_tcp(
    dst: &Address<'_>,
) -> anyhow::Result<(
    Option<SocketAddr>,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
)> {
    let stream = TcpStream::connect(dst)
        .timeout(Duration::from_secs(2))
        .await
        .ok_or_else(|| anyhow!("Timeout connecting to {dst}"))??;
    Ok((stream.local_addr().ok(), stream))
}
