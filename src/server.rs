use crate::geoip::{choose_country, choose_ip_addr, resolve_with_countries};
use futures_lite::{AsyncRead, AsyncWrite};
use smol::net::TcpListener;
use smol::spawn;

use crate::io::TcpStream;
use crate::proxy::protocol::{ProxyRequest, ProxyRequestType as rt, ProxyResult};
use crate::proxy::tcp::serve_tcp_proxy;
use crate::proxy::udp::serve_udp_proxy;
use crate::utils::{read_json_lengthed_async, write_json_lengthed_async};

pub async fn serve_client(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let mut stream = super::cipher::server::listen(stream).await?;
    let ProxyRequest { t, reject, accept }: ProxyRequest =
        read_json_lengthed_async(&mut stream).await?;
    

    match t {
        rt::SocksTCP(addr) => {
            let original_addrs = resolve_with_countries(&addr)
                .await;
            
            let addrs = original_addrs
                .iter()
                .filter(|(addr, country)| {
                    choose_country(country.as_ref(), accept.as_slice(), reject.as_slice())
                })
                .map(|(addr, _)| addr.clone())
                .collect::<Vec<_>>();
            if addrs.is_empty() {
                write_json_lengthed_async(&mut stream, ProxyResult::ErrHostRejected {})
            }
            serve_tcp_proxy(addrs.as_slice(), stream).await
        }
        rt::Http(req) => serve_http_proxy(req, stream).await,
        rt::SocksUDP => serve_udp_proxy(stream).await,
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
