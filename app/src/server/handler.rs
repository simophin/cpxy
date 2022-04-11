use futures_lite::{AsyncRead, AsyncWrite};
use smol::spawn;

use super::{dns::resolve_domains, tcp::serve_http_proxy, tcp::serve_tcp_proxy};
use crate::io::TcpListener;
use crate::proxy::protocol::ProxyRequest;
use crate::utils::read_bincode_lengthed_async;

pub async fn serve_client(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let mut stream = crate::cipher::server::listen(stream).await?;
    let req: ProxyRequest = read_bincode_lengthed_async(&mut stream).await?;
    log::debug!("Receive client request: {req:?}");

    match req {
        ProxyRequest::TCP { dst } => serve_tcp_proxy(&dst, stream).await,
        ProxyRequest::HTTP { dst, https, req } => serve_http_proxy(https, &dst, req, stream).await,
        ProxyRequest::UDP { initial_data } => {
            todo!()
        }
        ProxyRequest::DNS { domains } => resolve_domains(domains, stream).await,
    }
}

pub async fn run_server(listener: TcpListener) -> anyhow::Result<()> {
    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Accepted client {addr}");
        spawn(async move {
            if let Err(e) = serve_client(stream).await {
                log::error!("Error serving client {addr}: {e:?}");
            }
            log::info!("Client {addr} disconnected");
        })
        .detach();
    }
}
