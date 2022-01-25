use futures_lite::{AsyncRead, AsyncWrite};
use smol::net::TcpListener;
use smol::spawn;

use crate::http::serve_http_proxy;
use crate::proxy::handler::{receive_proxy_request, ProxyRequest};
use crate::proxy::tcp::serve_tcp_proxy;
use crate::proxy::udp::serve_udp_proxy;

pub async fn serve_client(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let mut stream = super::cipher::server::listen(stream).await?;
    match receive_proxy_request(&mut stream).await? {
        ProxyRequest::SocksTCP(addr) => serve_tcp_proxy(addr, stream).await,
        ProxyRequest::Http(req) => serve_http_proxy(req, stream).await,
        ProxyRequest::SocksUDP => serve_udp_proxy(stream).await,
    }
}

pub async fn run_server(listener: TcpListener) -> anyhow::Result<()> {
    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Accepted client {addr}");
        spawn(async move {
            if let Err(e) = serve_client(stream).await {
                log::error!("Error serving client {addr}: {e}");
            }
            log::info!("Client {addr} disconnected");
        })
        .detach();
    }
}
