use crate::cipher::strategy::EncryptionStrategy;
use crate::fetch::HttpStream;
use crate::socks5::Address;
use crate::utils::{read_bincode_lengthed_async, write_bincode_lengthed};
use futures_lite::{AsyncRead, AsyncWrite};
use protocol::{ProxyRequest, ProxyResult};
use std::time::{Duration, Instant};

pub mod protocol;
pub mod udp;

pub async fn request_proxy_upstream_http(
    stream: HttpStream<impl AsyncRead + AsyncWrite + Unpin + Send + Sync>,
    req: &ProxyRequest<'_>,
) -> anyhow::Result<(
    ProxyResult,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    Duration,
)> {
    let send_enc = EncryptionStrategy::pick_send(req, matches!(stream, HttpStream::SSL(_, _)));
    let receive_enc = EncryptionStrategy::pick_receive(req);

    log::debug!("EncryptionStrategy(send = {send_enc:?}, receive = {receive_enc})");

    let mut header = Vec::new();
    write_bincode_lengthed(&mut header, req)?;

    let start = Instant::now();

    let mut upstream = super::cipher::client::connect(url, send_enc, receive_enc, header).await?;

    Ok((
        read_bincode_lengthed_async(&mut upstream).await?,
        upstream,
        start.elapsed(),
    ))
}

pub async fn request_proxy_upstream(
    upstream_tls: bool,
    upstream_address: &Address<'_>,
    req: &ProxyRequest<'_>,
) -> anyhow::Result<(
    ProxyResult,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    Duration,
)> {
    let send_enc = EncryptionStrategy::pick_send(&req, upstream_tls);
    let receive_enc = EncryptionStrategy::pick_receive(&req);

    log::debug!("EncryptionStrategy(send = {send_enc:?}, receive = {receive_enc})");

    let mut header = Vec::new();
    write_bincode_lengthed(&mut header, req)?;

    let start = Instant::now();

    let url = format!(
        "{}://{upstream_address}",
        if upstream_tls { "https" } else { "http" }
    );

    let mut upstream = super::cipher::client::connect(url, send_enc, receive_enc, header).await?;

    Ok((
        read_bincode_lengthed_async(&mut upstream).await?,
        upstream,
        start.elapsed(),
    ))
}
