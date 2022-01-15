use crate::http;
use crate::http::Request;
use anyhow::anyhow;
use async_std::io::copy;
use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use futures::{select, AsyncReadExt, FutureExt};
use std::net::SocketAddr;

async fn prepare_client_tcp(upstream_addr: String) -> anyhow::Result<(TcpStream, SocketAddr)> {
    let upstream = TcpStream::connect(&upstream_addr).await?;
    log::debug!("Connected to {upstream_addr}");
    let bound_addr = upstream.local_addr()?;
    Ok((upstream, bound_addr))
}

async fn serve_client_tcp(mut sock: TcpStream, address: String) -> anyhow::Result<()> {
    let (upstream, bound_addr) = match prepare_client_tcp(address).await {
        Ok(v) => v,
        Err(e) => {
            http::Response::write_rejection(e.to_string().as_str(), &mut sock).await?;
            return Err(e.into());
        }
    };

    http::Response::write(&bound_addr, &mut sock).await?;
    let (upstream_rx, upstream_tx) = upstream.split();
    let (rx, tx) = sock.split();
    select! {
        r1 = copy(upstream_rx, tx).fuse() => r1?,
        r2 = copy(rx, upstream_tx).fuse() => r2?,
    };
    Ok(())
}

async fn serve_client_udp(sock: TcpStream, address: String) -> anyhow::Result<()> {
    unimplemented!()
}

async fn serve_client(mut sock: TcpStream) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 4096];

    let Request { protocol, address } = {
        let mut n = 0;
        loop {
            match sock.read(&mut buf.as_mut_slice()[n..]).await? {
                0 => return Err(anyhow!("Unexpected EOF")),
                v => n += v,
            };

            if let Some(v) = http::Request::parse(&buf.as_slice()[..n])? {
                break v;
            }
        }
    };

    match protocol.as_ref() {
        "tcp" => serve_client_tcp(sock, address).await,
        "udp" => serve_client_udp(sock, address).await,
        _ => http::Response::write_rejection("Unknown protocol", &mut sock).await,
    }
}

pub async fn run_server(bind_addr: &str) -> anyhow::Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;
    log::info!("Started listening on {bind_addr}");
    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!("Accepted connection from {addr}");
        spawn(async move {
            if let Err(e) = serve_client(sock).await {
                log::error!("Error serving client {addr}: {e}");
            }
            log::info!("Closing connection to {addr}");
        });
    }
}