use crate::fetch::send_http;
use crate::http::HttpRequest;
use crate::io::TcpStream;
use crate::proxy::protocol::ProxyResult;
use crate::socks5::Address;
use crate::utils::{copy_duplex, write_bincode_lengthed_async};
use anyhow::anyhow;
use futures_lite::{AsyncRead, AsyncWrite};
use futures_util::FutureExt;
use smol_timeout::TimeoutExt;
use std::net::SocketAddr;
use std::time::Duration;

async fn prepare(target: &Address) -> anyhow::Result<(Option<SocketAddr>, TcpStream)> {
    let socket = TcpStream::connect(target).await?;
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
            write_bincode_lengthed_async(&mut src, &ProxyResult::ErrGeneric { msg: e.to_string() })
                .await?;
            return Err(e);
        }
    };

    copy_duplex(upstream, src, None, None).await
}

pub async fn serve_tcp_proxy(
    target: &Address,
    src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    log::info!("Proxying upstream: tcp://{target:?}");
    serve_tcp_proxy_common(prepare(target).timeout(Duration::from_secs(3)).await, src).await
}

pub async fn serve_http_proxy(
    req: HttpRequest<'_>,
    src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    serve_tcp_proxy_common(
        send_http(req, None)
            .map(|r| r.map(|(socket, _)| (None, socket)))
            .timeout(Duration::from_secs(3))
            .await,
        src,
    )
    .await
}
