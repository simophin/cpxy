use crate::geoip::resolve_with_countries;
use futures_lite::{AsyncRead, AsyncWrite};
use smol::net::TcpListener;
use smol::spawn;
use std::net::SocketAddr;

use crate::io::TcpStream;
use crate::proxy::protocol::{IPPolicy, ProxyRequest, ProxyRequestType as rt, ProxyResult};
use crate::proxy::tcp::{serve_http_proxy, serve_tcp_proxy};
use crate::proxy::udp::serve_udp_proxy;
use crate::socks5::Address;
use crate::utils::{read_bincode_lengthed_async, write_bincode_lengthed_async};

async fn check_resolve_addresses(
    addr: &Address,
    policy: &IPPolicy,
) -> Result<Vec<SocketAddr>, ProxyResult> {
    let original_addrs = resolve_with_countries(addr).await;

    let mut addrs = original_addrs
        .iter()
        .filter(|(addr, country)| policy.should_keep(&addr.ip(), *country))
        .map(|(addr, c)| (addr.clone(), *c))
        .collect::<Vec<_>>();

    policy.sort_by_preferences(&mut addrs);

    log::debug!("Looking up {addr} with policy {policy:?}, result: {addrs:?}");

    if addrs.is_empty() {
        return Err(ProxyResult::ErrHostRejected {
            resolved: original_addrs
                .into_iter()
                .map(|(addr, cc)| (addr.ip(), cc))
                .collect(),
        });
    }

    Ok(addrs.into_iter().map(|(addr, _)| addr).collect())
}

pub async fn serve_client(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let mut stream = super::cipher::server::listen(stream).await?;
    let ProxyRequest { t, policy }: ProxyRequest = read_bincode_lengthed_async(&mut stream).await?;

    match t {
        rt::SocksTCP(addr) => match check_resolve_addresses(&addr, &policy).await {
            Ok(addrs) => serve_tcp_proxy(addrs.as_slice(), stream).await,
            Err(e) => {
                write_bincode_lengthed_async(&mut stream, &e).await?;
                return Err(e.into());
            }
        },
        rt::Http(addr, headers) => match check_resolve_addresses(&addr, &policy).await {
            Ok(addrs) => serve_http_proxy(addrs.as_slice(), headers.as_slice(), stream).await,
            Err(e) => {
                write_bincode_lengthed_async(&mut stream, &e).await?;
                return Err(e.into());
            }
        },
        rt::SocksUDP(addr) if addr.is_some() => {
            match check_resolve_addresses(&addr.unwrap(), &policy).await {
                Ok(_) => serve_udp_proxy(stream, true).await,
                Err(e) => {
                    write_bincode_lengthed_async(&mut stream, &e).await?;
                    return Err(e.into());
                }
            }
        }
        rt::SocksUDP(_) => serve_udp_proxy(stream, true).await,
    }
}

pub async fn run_server(listener: TcpListener) -> anyhow::Result<()> {
    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Accepted client {addr}");
        spawn(async move {
            if let Err(e) = serve_client(TcpStream::from(stream)).await {
                log::error!("Error serving client {addr}: {e}");
            }
            log::info!("Client {addr} disconnected");
        })
        .detach();
    }
}
