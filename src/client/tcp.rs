use std::time::Duration;

use anyhow::Context;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite};
use smol_timeout::TimeoutExt;

use crate::{
    config::ClientConfig,
    handshake::Handshaker,
    socks5::Address,
    utils::{copy_duplex, new_vec_uninitialised, VecExt},
};

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

const TCP_PROXY_PRE_READ_TIMEOUT: Duration = Duration::from_millis(200);

pub async fn serve_tcp_tproxy_conn(
    dst: Address<'_>,
    config: &ClientConfig,
    stats: &ClientStatistics,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<()> {
    let initial_data = match dst.get_port() {
        80 | 443 => {
            let mut vec = new_vec_uninitialised(4096);
            match stream
                .read(&mut vec)
                .timeout(TCP_PROXY_PRE_READ_TIMEOUT)
                .await
            {
                Some(Ok(size)) => {
                    vec.set_len_uninit(size);
                    Some(vec)
                }
                Some(Err(e)) => return Err(e.into()),
                None => None,
            }
        }
        _ => None,
    };

    let upstream = find_and_connect_stream(
        &dst,
        initial_data.as_ref().map(|s| s.as_ref()),
        config,
        stats,
    )
    .await
    .with_context(|| format!("Finding proxy for tcp://{dst}"))?;

    copy_duplex(stream, upstream, None, None).await
}
