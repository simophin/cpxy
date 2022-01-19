use anyhow::anyhow;
use std::fmt::Display;
use std::sync::Arc;
use tokio::io::{split, AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::handshake::{handshake_socks5_or_http, ProxyRequest};
use crate::socks5::{Address, ClientConnRequest, ConnStatusCode};
use crate::utils::{copy_io, RWBuffer};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::{select, spawn};

pub async fn run_client(
    bind_addr: impl ToSocketAddrs + Display,
    upstream_host: &str,
    upstream_port: u16,
) -> anyhow::Result<()> {
    log::info!("Start client at {bind_addr}");
    let listener = TcpListener::bind(bind_addr).await?;
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
    upstream: String,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::default();
    let req = handshake_socks5_or_http(&mut socks, &mut buf).await?;
    log::info!("Proxying {req:?}");

    let upstream = super::proxy::handler::request_proxy(&req, move |buf| async move {
        let upstream = TcpStream::connect(upstream).await?;
        super::cipher::client::connect(upstream, buf).await
    })
    .await;

    let (upstream_r, upstream_w) = match (req, upstream) {
        (ProxyRequest::SocksTCP(_), Ok((bound, upstream))) => {
            ClientConnRequest::respond(&mut socks, ConnStatusCode::GRANTED, &Address::IP(bound))
                .await?;
            split(upstream)
        }
        (ProxyRequest::SocksTCP(_) | ProxyRequest::SocksUDP(_), Err(e)) => {
            let code = e
                .downcast_ref::<ConnStatusCode>()
                .cloned()
                .unwrap_or(ConnStatusCode::FAILED);
            ClientConnRequest::respond(&mut socks, code, &Default::default()).await?;
            return Err(e);
        }
        (ProxyRequest::Http(_), Ok((_, upstream))) => split(upstream),
        (ProxyRequest::Http(_), Err(e)) => {
            socks
                .write_all(b"HTTP/1.1 500 Internal error\r\n\r\n")
                .await?;
            return Err(e);
        }
        _ => {
            todo!()
        }
    };

    let (r, w) = split(socks);
    select! {
        r1 = copy_io(r, upstream_w) => r1,
        r2 = copy_io(upstream_r, w) => r2,
    }
}
