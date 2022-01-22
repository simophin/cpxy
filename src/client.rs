use std::sync::mpsc::Receiver;
use std::time::Duration;

use crate::cipher::strategy::EncryptionStrategy;
use crate::handshake::Handshaker;
use crate::proxy::handler::ProxyRequest;
use crate::utils::{copy_duplex, RWBuffer};
use async_std::future::timeout;
use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use futures_lite::{AsyncRead, AsyncWrite};
use futures_util::{select, FutureExt};

pub async fn run_client(
    listener: TcpListener,
    upstream_host: &str,
    upstream_port: u16,
    mut quit_rx: async_broadcast::Receiver<()>,
) -> anyhow::Result<()> {
    let upstream = format!("{upstream_host}:{upstream_port}");

    loop {
        let (sock, addr) = select! {
            v = listener.accept().fuse() => v?,
            _ = quit_rx.recv().fuse() => return Ok(()),
        };
        log::info!("Accepted client from: {addr}");

        let upstream = upstream.clone();
        let mut quit_rx = quit_rx.clone();
        spawn(async move {
            select! {
                r1 = serve_proxy_client(sock, upstream, quit_rx.clone()).fuse() => {
                    if let Err(e) = r1 {
                         log::error!("Error serving client {addr}: {e}");
                    }
                },
                _ = quit_rx.recv().fuse() => {},
            };
            log::info!("Client {addr} disconnected");
        });
    }
}

async fn serve_proxy_client(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    upstream_addr: String,
    quit_rx: Receiver<()>,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::default();
    let (handshaker, req) = Handshaker::start(&mut socks, &mut buf).await?;
    log::info!("Requesting to proxy {req:?}");

    let send_enc = EncryptionStrategy::pick_send(&req);
    let receive_enc = EncryptionStrategy::pick_receive(&req);

    let r = super::proxy::handler::request_proxy(&req, move |buf| async move {
        let upstream =
            timeout(Duration::from_secs(2), TcpStream::connect(&upstream_addr)).await??;
        super::cipher::client::connect(upstream, upstream_addr.as_str(), send_enc, receive_enc, buf)
            .await
    })
    .await;

    let upstream = match r {
        Ok((proxy_r, upstream)) => {
            handshaker.respond(&mut socks, Ok(proxy_r)).await?;
            match req {
                ProxyRequest::SocksTCP(_) | ProxyRequest::Http(_) => upstream,
                ProxyRequest::SocksUDP(_) => todo!(),
            }
        }
        Err(err) => {
            handshaker.respond(&mut socks, Err(())).await?;
            return Err(err);
        }
    };

    copy_duplex(upstream, socks).await
}
