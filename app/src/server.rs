use futures_lite::{AsyncRead, AsyncWrite};
use smol::spawn;

use crate::io::{TcpListener, TcpStream};
use crate::proxy::protocol::ProxyRequest;
use crate::proxy::tcp::{serve_http_proxy, serve_tcp_proxy};
use crate::proxy::udp::serve_udp_proxy;
use crate::utils::read_bincode_lengthed_async;

pub async fn serve_client(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let mut stream = super::cipher::server::listen(stream).await?;
    let req: ProxyRequest = read_bincode_lengthed_async(&mut stream).await?;
    log::debug!("Receive client request: {req:?}");

    match req {
        ProxyRequest::TCP { dst } => serve_tcp_proxy(&dst, stream).await,
        ProxyRequest::Http { dst, request } => {
            serve_http_proxy(&dst, request.as_ref(), stream).await
        }
        ProxyRequest::UDP => serve_udp_proxy(stream, true).await,
    }
}

pub async fn run_server(listener: TcpListener) -> anyhow::Result<()> {
    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Accepted client {addr}");
        spawn(async move {
            if let Err(e) = serve_client(TcpStream::from(stream)).await {
                log::error!("Error serving client {addr}: {e:?}");
            }
            log::info!("Client {addr} disconnected");
        })
        .detach();
    }
}
