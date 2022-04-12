use crate::cipher::strategy::EncryptionStrategy;
use crate::config::UpstreamConfig;
use crate::utils::{read_bincode_lengthed_async, write_bincode_lengthed};
use futures_lite::{AsyncRead, AsyncWrite};
use protocol::{ProxyRequest, ProxyResult};
use std::time::{Duration, Instant};

pub mod protocol;
pub mod udp;

pub async fn request_proxy_upstream(
    c: &UpstreamConfig,
    req: &ProxyRequest<'_>,
) -> anyhow::Result<(
    ProxyResult,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    Duration,
)> {
    let send_enc = EncryptionStrategy::pick_send(&req, c.tls);
    let receive_enc = EncryptionStrategy::pick_receive(&req);

    log::debug!("EncryptionStrategy(send = {send_enc:?}, receive = {receive_enc})");

    let mut header = Vec::new();
    write_bincode_lengthed(&mut header, req)?;

    let start = Instant::now();

    let url = format!("{}://{}", if c.tls { "https" } else { "http" }, c.address);

    let mut upstream = super::cipher::client::connect(url, send_enc, receive_enc, header).await?;

    Ok((
        read_bincode_lengthed_async(&mut upstream).await?,
        upstream,
        start.elapsed(),
    ))
}
