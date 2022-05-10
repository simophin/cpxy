use anyhow::Context;
use futures::{AsyncRead, AsyncWrite};

use crate::{config::ClientConfig, handshake::Handshaker, socks5::Address, utils::copy_duplex};

use super::{common::find_and_connect_stream, ClientStatistics};

pub async fn serve_tcp_proxy_conn(
    dst: Address<'_>,
    config: &ClientConfig,
    stats: &ClientStatistics,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let upstream = match find_and_connect_stream(&dst, None, config, stats)
        .await
        .with_context(|| format!("Finding proxy for tcp://{dst}"))
    {
        Ok(v) => {
            handshaker.respond_ok(&mut stream, None).await?;
            v
        }
        Err(e) => {
            handshaker.respond_err(&mut stream).await?;
            return Err(e);
        }
    };

    copy_duplex(stream, upstream, None, None).await
}
