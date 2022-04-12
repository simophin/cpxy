use anyhow::{anyhow, Context};
use futures_lite::{AsyncRead, AsyncWrite};

use crate::{
    config::ClientConfig, fetch::send_http, handshake::Handshaker, http::HttpRequest,
    proxy::protocol::ProxyRequest, socks5::Address, utils::copy_duplex,
};

use super::{utils::request_best_upstream, ClientStatistics};

pub async fn serve_http_proxy_conn(
    dst: Address<'_>,
    https: bool,
    req: HttpRequest<'_>,
    config: &ClientConfig,
    stats: &ClientStatistics,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let proxy_request = ProxyRequest::HTTP {
        dst: dst.clone(),
        https,
        req,
    };
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
        if let ProxyRequest::HTTP { req, .. } = proxy_request {
            match send_http(https, &dst, &req).await {
                Ok(upstream) => {
                    handshaker.respond_ok(&mut stream, None).await?;
                    copy_duplex(upstream, stream, None, None)
                        .await
                        .context("Redirecting upstream traffic")
                }
                Err(e) => {
                    handshaker.respond_err(&mut stream).await?;
                    Err(e)
                }
            }
        } else {
            panic!("Proxy request should be HTTP");
        }
    } else {
        handshaker.respond_err(&mut stream).await?;
        Err(anyhow!("No where to direct TCP traffic"))
    }
}
