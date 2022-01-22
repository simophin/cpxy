use crate::proxy::handler::ProxyResult;
use crate::socks5::Address;
use crate::utils::copy_duplex;
use anyhow::anyhow;
use async_std::future::timeout;
use async_std::net::TcpStream;
use futures_lite::{AsyncRead, AsyncWrite};
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
    let upstream = match timeout(Duration::from_secs(3), prepare(&target)).await {
        Ok(Ok((socket, addr))) => {
            super::handler::send_proxy_result(
                &mut src,
                ProxyResult::Granted {
                    bound_address: addr,
                },
            )
            .await?;
            socket
        }
        Err(_) => {
            super::handler::send_proxy_result(&mut src, ProxyResult::ErrTimeout).await?;
            return Err(anyhow!("Timeout waiting {target}"));
        }
        Ok(Err(e)) => {
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
