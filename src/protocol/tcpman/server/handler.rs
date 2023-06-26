use crate::protocol::direct::Direct;
use crate::protocol::Protocol;
use anyhow::Context;
use async_shutdown::Shutdown;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::spawn;

use super::{super::cipher, super::proto};
use crate::utils::copy_duplex;

pub async fn serve_client<P: Protocol + Send + Sync>(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    upstream_factory: impl FnOnce(&proto::Request) -> anyhow::Result<P>,
) -> anyhow::Result<()> {
    let (initial_data, hs) = cipher::server::accept_client(stream)
        .await
        .context("Awaiting handshake")?;

    let initial_data = initial_data.unwrap_or_default();

    let req = match proto::Request::parse(&initial_data).context("Parsing TCPMan request") {
        Ok(v) => v,
        Err(e) => {
            hs.respond_error(&e).await?;
            return Err(e);
        }
    };

    let upstream_protocol = match upstream_factory(&req) {
        Ok(v) => v,
        Err(e) => {
            hs.respond_error(&e).await?;
            return Err(e);
        }
    };

    match req {
        proto::Request::TCP { dst, initial_data } => {
            let upstream = match upstream_protocol
                .new_stream(&dst, Some(initial_data), &Default::default(), None)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    hs.respond_error(&e).await?;
                    return Err(e);
                }
            };

            copy_duplex(hs.respond_success().await?, upstream, None, None).await
        }
    }
}

pub async fn run_server(shutdown: Shutdown, listener: TcpListener) -> anyhow::Result<()> {
    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Accepted client {addr}");
        let shutdown = shutdown.clone();
        spawn(async move {
            if let Some(Err(e)) = shutdown
                .wrap_cancel(serve_client(stream, |_| Ok(Direct {})))
                .await
            {
                log::error!("Error serving client {addr}: {e:?}");
            }
            log::info!("Client {addr} disconnected");
        });
    }
}
