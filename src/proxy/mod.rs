use crate::cipher::strategy::EncryptionStrategy;
use crate::config::UpstreamConfig;
use crate::fetch::connect_http;
use crate::io::TcpStreamExt;
use crate::url::HttpUrl;
use crate::utils::{read_bincode_lengthed_async, write_bincode_lengthed};
use anyhow::Context;
use futures_lite::{AsyncRead, AsyncWrite};
use protocol::{ProxyRequest, ProxyResult};
use std::time::{Duration, Instant};

pub mod protocol;
pub mod udp;

pub async fn request_proxy_upstream_http(
    upstream_url: &HttpUrl<'_>,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    req: &ProxyRequest<'_>,
) -> anyhow::Result<(
    ProxyResult,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    Duration,
)> {
    let send_enc = EncryptionStrategy::pick_send(req, upstream_url.is_https);
    let receive_enc = EncryptionStrategy::pick_receive(req);

    log::debug!("EncryptionStrategy(send = {send_enc:?}, receive = {receive_enc})");

    let mut header = Vec::new();
    write_bincode_lengthed(&mut header, req)?;

    let start = Instant::now();

    let mut upstream =
        super::cipher::client::connect(upstream_url, stream, send_enc, receive_enc, header).await?;

    Ok((
        read_bincode_lengthed_async(&mut upstream).await?,
        upstream,
        start.elapsed(),
    ))
}

pub async fn request_proxy_upstream_with_config(
    fwmark: Option<u32>,
    upstream: &UpstreamConfig,
    req: &ProxyRequest<'_>,
) -> anyhow::Result<(
    ProxyResult,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    Duration,
)> {
    let stream = connect_http(upstream.tls, &upstream.address).await?;
    if let Some(fwmark) = fwmark {
        stream
            .inner()
            .set_sock_mark(fwmark)
            .context("Setting FWMARK")?;
    }

    let url = HttpUrl {
        is_https: upstream.tls,
        address: upstream.address.clone(),
        path: Default::default(),
    };
    request_proxy_upstream(&url, stream, req).await
}

pub async fn request_proxy_upstream(
    upstream_url: &HttpUrl<'_>,
    upstream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    req: &ProxyRequest<'_>,
) -> anyhow::Result<(
    ProxyResult,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    Duration,
)> {
    let send_enc = EncryptionStrategy::pick_send(&req, upstream_url.is_https);
    let receive_enc = EncryptionStrategy::pick_receive(&req);

    log::debug!("EncryptionStrategy(send = {send_enc:?}, receive = {receive_enc})");

    let mut header = Vec::new();
    write_bincode_lengthed(&mut header, req)?;

    let start = Instant::now();

    let mut upstream =
        super::cipher::client::connect(upstream_url, upstream, send_enc, receive_enc, header)
            .await?;

    Ok((
        read_bincode_lengthed_async(&mut upstream).await?,
        upstream,
        start.elapsed(),
    ))
}
