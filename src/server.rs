use crate::http::serve_http_proxy;
use crate::proxy::handler::{receive_proxy_request, ProxyRequest};
use crate::proxy::tcp::serve_tcp_proxy;
use futures_lite::{AsyncRead, AsyncWrite};
use smol::net::{AsyncToSocketAddrs, TcpListener};
use smol::spawn;
use std::fmt::Display;

pub async fn serve_client(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let mut stream = super::cipher::server::listen(stream).await?;
    match receive_proxy_request(&mut stream).await? {
        ProxyRequest::SocksTCP(addr) => serve_tcp_proxy(addr, stream).await,
        ProxyRequest::Http(req) => serve_http_proxy(req, stream).await,
        _ => todo!(),
    }
}

pub async fn run_server(listen_address: impl AsyncToSocketAddrs + Display) -> anyhow::Result<()> {
    log::info!("Server listening at {listen_address}");
    let listener = TcpListener::bind(listen_address).await?;
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
