use anyhow::Context;
use async_native_tls::TlsConnector;
use futures::{AsyncRead, AsyncWrite, TryFutureExt};

use crate::{
    config::ClientConfig, handshake::Handshaker, http::HttpRequest, socks5::Address,
    utils::copy_duplex,
};

use super::{common::find_and_connect_stream, ClientStatistics};

pub async fn serve_http_proxy_conn(
    dst: Address<'_>,
    https: bool,
    req: HttpRequest<'_>,
    config: &ClientConfig,
    stats: &ClientStatistics,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    if https {
        let upstream = match find_and_connect_tls(&dst, config, stats)
            .and_then(move |mut upstream| async move {
                req.to_async_writer(&mut upstream).await?;
                Ok(upstream)
            })
            .await
        {
            Ok(s) => {
                handshaker.respond_ok(&mut stream, None).await?;
                s
            }
            Err(e) => {
                handshaker.respond_err(&mut stream).await?;
                return Err(e);
            }
        };
        copy_duplex(stream, upstream, None, None).await
    } else {
        let upstream = match find_and_connect_stream(
            &dst,
            Some(req.to_builder().finalise().as_slice()),
            config,
            stats,
        )
        .await
        {
            Ok(s) => {
                handshaker.respond_ok(&mut stream, None).await?;
                s
            }
            Err(e) => {
                handshaker.respond_err(&mut stream).await?;
                return Err(e);
            }
        };
        copy_duplex(stream, upstream, None, None).await
    }
}

async fn find_and_connect_tls(
    dst: &Address<'_>,
    config: &ClientConfig,
    stats: &ClientStatistics,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync> {
    let upstream = find_and_connect_stream(&dst, None, config, stats)
        .await
        .context("Connecting to upstream")?;

    TlsConnector::new()
        .connect(dst.get_host().as_ref(), upstream)
        .await
        .context("Connecting to TLS stream")
}
