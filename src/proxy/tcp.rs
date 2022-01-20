use crate::proxy::handler::ProxyResult;
use crate::socks5::Address;
use crate::utils::copy_io;
use anyhow::anyhow;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{split, AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::select;
use tokio::time::timeout;

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
    mut src: impl AsyncRead + AsyncWrite + Unpin,
) -> anyhow::Result<()> {
    log::info!("Proxying upstream: {target}");
    let (upstream_r, upstream_w) = match timeout(Duration::from_secs(3), prepare(&target)).await {
        Ok(Ok((socket, addr))) => {
            super::handler::send_proxy_result(
                &mut src,
                ProxyResult::Granted {
                    bound_address: addr,
                },
            )
            .await?;
            split(socket)
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

    let (r, w) = split(src);
    select! {
        r1 = copy_io(r, upstream_w) => r1,
        r2 = copy_io(upstream_r, w) => r2,
    }
}
