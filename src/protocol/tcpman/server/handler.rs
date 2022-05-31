use crate::protocol::direct::Direct;
use crate::protocol::tcpman::dgram::{create_udp_sink, create_udp_stream};
use crate::protocol::Protocol;
use anyhow::Context;
use async_net::TcpListener;
use bytes::Bytes;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, StreamExt};
use smol::spawn;

use super::{super::cipher, super::proto};
use crate::utils::{copy_duplex, race};

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
        proto::Request::UDP { dst, initial_data } => {
            let (upstream_sink, upstream_stream) = match upstream_protocol
                .new_datagram(
                    &dst,
                    Bytes::copy_from_slice(initial_data),
                    &Default::default(),
                    None,
                )
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    hs.respond_error(&e).await?;
                    return Err(e);
                }
            };

            let (r, w) = hs.respond_success().await?.split();

            let task1 = spawn(create_udp_stream(r, Some(dst)).forward(upstream_sink));
            let task2 = spawn(upstream_stream.forward(create_udp_sink(w)));

            race(task1, task2).await
        }
    }
}

pub async fn run_server(listener: TcpListener) -> anyhow::Result<()> {
    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Accepted client {addr}");
        spawn(async move {
            if let Err(e) = serve_client(stream, |_| Ok(Direct {})).await {
                log::error!("Error serving client {addr}: {e:?}");
            }
            log::info!("Client {addr} disconnected");
        })
        .detach();
    }
}
