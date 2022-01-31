use crate::cipher::strategy::EncryptionStrategy;
use crate::config::UpstreamConfig;
use crate::io::TcpStream;
use crate::utils::{read_bincode_lengthed_async, write_bincode_lengthed};
use anyhow::anyhow;
use futures_lite::{AsyncRead, AsyncWrite};
use protocol::{ProxyRequest, ProxyResult};
use smol_timeout::TimeoutExt;
use std::time::Duration;

pub mod protocol;
pub mod tcp;
pub mod udp;

pub async fn request_proxy_upstream(
    c: &UpstreamConfig,
    req: &ProxyRequest,
) -> anyhow::Result<(
    ProxyResult,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
)> {
    let send_enc = EncryptionStrategy::pick_send(&req);
    let receive_enc = EncryptionStrategy::pick_receive(&req);

    log::debug!("EncryptionStrategy(send = {send_enc:?}, receive = {receive_enc})");

    let mut header = Vec::new();
    write_bincode_lengthed(&mut header, req)?;

    let upstream = TcpStream::connect(&c.address)
        .timeout(Duration::from_secs(3))
        .await
        .ok_or_else(|| anyhow!("Timeout connecting to {}", c.address))??;

    let mut upstream = super::cipher::client::connect(
        upstream,
        c.address.get_host().as_ref(),
        send_enc,
        receive_enc,
        header,
    )
    .await?;

    Ok((read_bincode_lengthed_async(&mut upstream).await?, upstream))
}
