use anyhow::Context;
use tokio::io::{copy_bidirectional, AsyncBufRead, AsyncReadExt, AsyncWrite};

use crate::config::ClientConfig;
use crate::handshaker;
use crate::protocol::ProxyRequest;

use super::{common::find_and_connect_stream, ClientStatistics};

pub async fn serve_tcp_proxy_conn<S>(
    req: ProxyRequest,
    handshaker: handshaker::Handshaker<S>,
    config: &ClientConfig,
    stats: &ClientStatistics,
) -> anyhow::Result<()>
where
    S: AsyncBufRead + AsyncWrite + Unpin,
{
    let (mut stream, mut upstream) = match find_and_connect_stream(&req, config, stats)
        .await
        .with_context(|| format!("Finding proxy for tcp://{}", req.dst))
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
