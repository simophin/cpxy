use crate::http;
use crate::http::Request;
use crate::udp::{copy_frame_to_udp, copy_udp_to_frame};
use crate::utils::copy_io;
use anyhow::anyhow;
use std::net::SocketAddr;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::select;
use tokio::task::spawn;

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
    let (upstream_rx, upstream_tx) = upstream.into_split();
    let (rx, tx) = sock.into_split();
    select! {
        r1 = copy_io(upstream_rx, tx) => r1,
        r2 = copy_io(rx, upstream_tx) => r2,
    }
}

async fn prepare_client_udp(address: &str) -> anyhow::Result<(UdpSocket, SocketAddr)> {
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    socket.connect(address).await?;
    let socket = socket.into_std()?;
    let remote_addr = socket.peer_addr()?;
    Ok((UdpSocket::from_std(socket)?, remote_addr))
}

async fn serve_client_udp(mut sock: TcpStream, address: String) -> anyhow::Result<()> {
    let (upstream, address) = match prepare_client_udp(&address).await {
        Ok(v) => v,
        Err(e) => {
            http::Response::write_rejection(e.to_string().as_str(), &mut sock).await?;
            return Err(e.into());
        }
    };

    http::Response::write(&address, &mut sock).await?;
    let (mut rx, mut tx) = sock.split();

    select! {
        r1 = copy_frame_to_udp(&mut rx, &upstream) => r1,
        r2 = copy_udp_to_frame(&upstream, &address, &mut tx) => r2,
    }
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
