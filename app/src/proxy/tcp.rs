use crate::io::TcpStream;
use crate::proxy::protocol::ProxyResult;
use crate::socks5::Address;
use crate::utils::{copy_duplex, write_bincode_lengthed_async};
use anyhow::anyhow;
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};
use smol_timeout::TimeoutExt;
use std::net::SocketAddr;
use std::time::Duration;

async fn prepare(target: &Address) -> anyhow::Result<(SocketAddr, TcpStream)> {
    let socket = TcpStream::connect(target).await?;
    Ok((socket.local_addr()?, socket))
}

async fn serve_tcp_proxy_common<'a>(
    upstream: Option<anyhow::Result<(SocketAddr, TcpStream)>>,
    mut src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'a,
) -> anyhow::Result<()> {
    let upstream = match upstream {
        Some(Ok((bound_address, socket))) => {
            write_bincode_lengthed_async(&mut src, &ProxyResult::Granted { bound_address }).await?;
            socket
        }
        None => {
            write_bincode_lengthed_async(&mut src, &ProxyResult::ErrTimeout).await?;
            return Err(anyhow!("Timeout waiting for upstream"));
        }
        Some(Err(e)) => {
            write_bincode_lengthed_async(&mut src, &ProxyResult::ErrGeneric { msg: e.to_string() })
                .await?;
            return Err(e);
        }
    };

    copy_duplex(upstream, src, None, None).await
}

pub async fn serve_tcp_proxy<'a>(
    target: &Address,
    src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'a,
) -> anyhow::Result<()> {
    log::info!("Proxying upstream: tcp://{target:?}");
    serve_tcp_proxy_common(prepare(target).timeout(Duration::from_secs(3)).await, src).await
}

async fn prepare_http(target: &Address, headers: &[u8]) -> anyhow::Result<(SocketAddr, TcpStream)> {
    let (addr, mut stream) = prepare(target).await?;
    log::debug!(
        "Writing to {target:?}: \n{}",
        String::from_utf8_lossy(headers)
    );
    stream.write_all(headers).await?;
    Ok((addr, stream))
}

pub async fn serve_http_proxy<'a>(
    target: &Address,
    headers: &[u8],
    src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'a,
) -> anyhow::Result<()> {
    serve_tcp_proxy_common(
        prepare_http(target, headers)
            .timeout(Duration::from_secs(3))
            .await,
        src,
    )
    .await
}
