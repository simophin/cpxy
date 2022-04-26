use crate::io::TcpStreamExt;
use crate::rt::spawn;
use futures::{AsyncRead, AsyncWrite};

use super::{
    dns::resolve_domains, tcp::serve_http_proxy, tcp::serve_tcp_proxy, udp::serve_udp_proxy_conn,
};
use crate::proxy::protocol::ProxyRequest;
use crate::rt::net::TcpListener;
use crate::utils::read_bincode_lengthed_async;

pub async fn serve_client(
    v4: bool,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    //TODO
    // let mut stream = crate::cipher::server::listen(stream).await?;
    let req: ProxyRequest = read_bincode_lengthed_async(&mut stream).await?;
    log::debug!("Receive client request: {req:?}");

    match req {
        ProxyRequest::TCP { dst } => serve_tcp_proxy(&dst, stream).await,
        ProxyRequest::HTTP { dst, https, req } => serve_http_proxy(https, &dst, req, stream).await,
        ProxyRequest::UDP {
            initial_data,
            initial_dst,
        } => serve_udp_proxy_conn(v4, stream, initial_data, initial_dst).await,
        ProxyRequest::DNS { domains } => resolve_domains(domains, stream).await,
        ProxyRequest::EchoTestTcp => super::echo::serve_tcp(stream).await,
        ProxyRequest::EchoTestUdp => super::echo::serve_udp(stream).await,
    }
}

pub async fn run_server(listener: TcpListener) -> anyhow::Result<()> {
    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Accepted client {addr}");
        spawn(async move {
            if let Err(e) = serve_client(stream.is_v4(), stream).await {
                log::error!("Error serving client {addr}: {e:?}");
            }
            log::info!("Client {addr} disconnected");
        })
        .detach();
    }
}
