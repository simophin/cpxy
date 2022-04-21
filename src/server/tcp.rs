use crate::fetch::send_http;
use crate::http::HttpRequest;
use crate::io::connect_tcp;
use crate::proxy::protocol::ProxyResult;
use crate::rt::net::TcpStream;
use crate::rt::TimeoutExt;
use crate::socks5::Address;
use crate::utils::{copy_duplex, write_bincode_lengthed_async};
use anyhow::anyhow;
use futures::{AsyncRead, AsyncWrite};
use futures_util::FutureExt;
use std::net::SocketAddr;
use std::time::Duration;

async fn prepare(target: &Address<'_>) -> anyhow::Result<(Option<SocketAddr>, TcpStream)> {
    let socket = connect_tcp(target).await?;
    Ok((socket.local_addr().ok(), socket))
}

async fn serve_tcp_proxy_common(
    upstream: Option<
        anyhow::Result<(
            Option<SocketAddr>,
            impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
        )>,
    >,
    mut src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let upstream = match upstream {
        Some(Ok((bound_address, socket))) => {
            write_bincode_lengthed_async(
                &mut src,
                &ProxyResult::Granted {
                    bound_address,
                    solved_addresses: None,
                },
            )
            .await?;
            socket
        }
        None => {
            write_bincode_lengthed_async(&mut src, &ProxyResult::ErrTimeout).await?;
            return Err(anyhow!("Timeout waiting for upstream"));
        }
        Some(Err(e)) => {
            log::error!("Error connecting to upstream: {e:?}");
            write_bincode_lengthed_async(&mut src, &ProxyResult::ErrGeneric { msg: e.to_string() })
                .await?;
            return Err(e);
        }
    };

    copy_duplex(upstream, src, None, None).await
}

pub async fn serve_tcp_proxy(
    target: &Address<'_>,
    src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    log::info!("Proxying upstream: tcp://{target:?}");
    serve_tcp_proxy_common(prepare(target).timeout(Duration::from_secs(3)).await, src).await
}

pub async fn serve_http_proxy(
    https: bool,
    dst: &Address<'_>,
    req: HttpRequest<'_>,
    src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    log::info!("Proxying upstream: http(s)://{dst}");
    serve_tcp_proxy_common(
        send_http(https, dst, &req)
            .map(|r| r.map(|socket| (None, socket)))
            .timeout(Duration::from_secs(3))
            .await,
        src,
    )
    .await
}
