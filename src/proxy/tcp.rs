use crate::proxy::handler::ProxyResult;
use crate::socks5::Address;
use crate::utils::copy_duplex;
use anyhow::anyhow;
use futures_lite::{AsyncRead, AsyncWrite};
use smol::net::TcpStream;
use smol_timeout::TimeoutExt;
use std::net::SocketAddr;
use std::time::Duration;

async fn prepare(target: &Address) -> anyhow::Result<(TcpStream, SocketAddr)> {
    let socket = match target {
        Address::IP(addr) => TcpStream::connect(addr).await?,
        p => TcpStream::connect(p.to_string()).await?,
    };

    let addr = socket.local_addr()?;
    Ok((socket, addr))
}

pub async fn serve_tcp_proxy(
    target: Address,
    mut src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    log::info!("Proxying upstream: {target}");
    let upstream = match prepare(&target).timeout(Duration::from_secs(3)).await {
        Some(Ok((socket, addr))) => {
            super::handler::send_proxy_result(
                &mut src,
                ProxyResult::Granted {
                    bound_address: addr,
                },
            )
            .await?;
            socket
        }
        None => {
            super::handler::send_proxy_result(&mut src, ProxyResult::ErrTimeout).await?;
            return Err(anyhow!("Timeout waiting {target}"));
        }
        Some(Err(e)) => {
            super::handler::send_proxy_result(
                &mut src,
                ProxyResult::ErrGeneric { msg: e.to_string() },
            )
            .await?;
            return Err(e);
        }
    };

    copy_duplex(upstream, src).await
}
