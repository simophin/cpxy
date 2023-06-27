use anyhow::Context;
use tokio::io::{copy_bidirectional, AsyncBufRead, AsyncRead, AsyncReadExt, AsyncWrite};

use crate::handshaker;
use crate::{config::ClientConfig, utils::VecExt};

use super::{common::find_and_connect_stream, ClientStatistics};

pub async fn serve_tcp_proxy_conn<S>(
    req: handshaker::Request,
    handshaker: handshaker::Handshaker<S>,
    config: &ClientConfig,
    stats: &ClientStatistics,
) -> anyhow::Result<()>
where
    S: AsyncBufRead + AsyncWrite + Unpin,
{
    let (mut stream, mut upstream) = match find_and_connect_stream(
        &req.address,
        req.initial_data.as_ref().map(|d| d.as_ref()),
        config,
        stats,
    )
    .await
    .with_context(|| format!("Finding proxy for tcp://{}", req.address))
    {
        Ok(upstream) => (handshaker.respond_ok().await?, upstream),
        Err(e) => {
            handshaker.respond_err(&e).await?;
            return Err(e);
        }
    };

    copy_bidirectional(&mut stream, &mut upstream)
        .await
        .context("Copying data")?;

    Ok(())
}
