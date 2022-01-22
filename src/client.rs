use std::fmt::Display;
use std::time::Duration;

use crate::cipher::strategy::EncryptionStrategy;
use crate::handshake::Handshaker;
use crate::proxy::handler::ProxyRequest;
use crate::utils::{copy_io, RWBuffer};
use async_std::io::timeout;
use async_std::net::{TcpListener, TcpStream, ToSocketAddrs};
use async_std::spawn;
use async_std::task::spawn;
use futures_lite::future::race;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncWrite};

pub async fn run_client(
    listen_address: impl ToSocketAddrs + Display,
    upstream_host: &str,
    upstream_port: u16,
) -> anyhow::Result<()> {
    log::info!("Start client at {listen_address}");
    let listener = TcpListener::bind(listen_address).await?;
    let upstream = format!("{upstream_host}:{upstream_port}");

    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!("Accepted client from: {addr}");

        let upstream = upstream.clone();
        spawn(async move {
            if let Err(e) = serve_proxy_client(sock, upstream).await {
                log::error!("Error serving client {addr}: {e}");
            }
            log::info!("Client {addr} disconnected");
        });
    }
}

async fn serve_proxy_client(
    mut socks: impl AsyncRead + AsyncWrite + Unpin,
    upstream_addr: String,
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

    let (upstream_r, upstream_w) = match r {
        Ok((proxy_r, upstream)) => {
            handshaker.respond(&mut socks, Ok(proxy_r)).await?;
            match req {
                ProxyRequest::SocksTCP(_) | ProxyRequest::Http(_) => split(upstream),
                ProxyRequest::SocksUDP(_) => todo!(),
            }
        }
        Err(err) => {
            handshaker.respond(&mut socks, Err(())).await?;
            return Err(err);
        }
    };

    let (r, w) = split(socks);

    let copy_task_1 = spawn(async move { copy_io(r, upstream_w).await });
    let copy_task_2 = spawn(async move { copy_io(upstream_r, w).await });

    race(copy_task_1, copy_task_2).await
}
