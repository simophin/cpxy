use crate::http;
use async_native_tls::TlsStream;
use async_std::net::{TcpListener, TcpStream, UdpSocket};
use async_std::task::spawn;
use futures::{select, AsyncRead, AsyncReadExt, AsyncWrite, FutureExt};
use std::net::SocketAddr;

use crate::socks5::{negotiate_request, Address, ClientConnRequest, Command, ConnStatusCode};
use crate::udp::{copy_frame_to_socks5_udp, copy_socks5_udp_to_frame};

async fn negotiate_with_server(
    upstream_host: &str,
    upstream_addr: &str,
    protocol: &str,
    address: &Address,
) -> anyhow::Result<(http::Response, TlsStream<TcpStream>)> {
    log::info!("Connecting to {}", upstream_addr);
    let mut upstream =
        async_native_tls::connect(upstream_host, TcpStream::connect(upstream_addr).await?).await?;
    log::debug!("Connected to {}", upstream_addr);

    http::Request::write(protocol, address, upstream_addr, &mut upstream).await?;

    let mut n = 0;
    let mut buf = vec![0u8; 4096];

    loop {
        match upstream.read(&mut buf.as_mut_slice()[n..]).await? {
            0 => return Err(anyhow::anyhow!("Unexpected EOF")),
            v => n += v,
        };

        if let Some(v) = http::Response::parse(&buf.as_slice()[..n])? {
            return Ok((v, upstream));
        }
    }
}

async fn serve_client_tcp(
    rx: impl AsyncRead + Unpin,
    mut tx: impl AsyncWrite + Unpin,
    upstream_host: &str,
    upstream_addr: &str,
    address: Address,
) -> anyhow::Result<()> {
    let (bound, upstream) =
        match negotiate_with_server(&upstream_host, &upstream_addr, "tcp", &address).await {
            Ok(v) => v,
            Err(e) => {
                ClientConnRequest::respond(&mut tx, ConnStatusCode::FAILED, &Default::default())
                    .await?;
                return Err(e.into());
            }
        };

    ClientConnRequest::respond(
        &mut tx,
        ConnStatusCode::GRANTED,
        &Address::IP(bound.bound_address),
    )
    .await?;

    let (upstream_rx, upstream_tx) = upstream.split();
    select! {
        r1 = async_std::io::copy(rx, upstream_tx).fuse() => r1?,
        r2 = async_std::io::copy(upstream_rx, tx).fuse() => r2?,
    };
    Ok(())
}

async fn prepare_client_udp(
    upstream_host: &str,
    upstream_addr: &str,
    address: &Address,
) -> anyhow::Result<(UdpSocket, TlsStream<TcpStream>, SocketAddr)> {
    let (_, upstream) =
        negotiate_with_server(&upstream_host, &upstream_addr, "udp", address).await?;

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let local_udp_addr = socket.local_addr()?;
    Ok((socket, upstream, local_udp_addr))
}

async fn serve_client_udp(
    mut tx: impl AsyncWrite + Unpin,
    upstream_host: &str,
    upstream_addr: &str,
    address: Address,
) -> anyhow::Result<()> {
    let (udp_socket, upstream, local_udp_addr) =
        match prepare_client_udp(upstream_host, upstream_addr, &address).await {
            Ok(v) => v,
            Err(e) => {
                ClientConnRequest::respond(&mut tx, ConnStatusCode::FAILED, &Default::default())
                    .await?;
                return Err(e);
            }
        };

    ClientConnRequest::respond(
        &mut tx,
        ConnStatusCode::GRANTED,
        &Address::IP(local_udp_addr),
    )
    .await?;

    let (mut upstream_rx, mut upstream_tx) = upstream.split();

    select! {
        r1 = copy_socks5_udp_to_frame(&udp_socket, &address, &mut upstream_tx).fuse() => r1,
        r2 = copy_frame_to_socks5_udp(&mut upstream_rx, &address, &udp_socket).fuse() => r2,
    }
}

async fn serve_client(
    sock: TcpStream,
    upstream_host: &str,
    upstream_addr: &str,
) -> anyhow::Result<()> {
    let (mut rx, mut tx) = sock.split();

    let ClientConnRequest { cmd, address } = match negotiate_request(&mut rx, &mut tx).await {
        Ok(r) => r,
        Err(e) if e.is::<ConnStatusCode>() => {
            ClientConnRequest::respond(&mut tx, *e.downcast_ref().unwrap(), &Default::default())
                .await?;
            return Err(e.into());
        }
        Err(e) => return Err(e.into()),
    };

    match cmd {
        Command::CONNECT_TCP => {
            serve_client_tcp(rx, tx, upstream_host, upstream_addr, address).await
        }

        Command::BIND_UDP => serve_client_udp(tx, upstream_host, upstream_addr, address).await,

        _ => {
            ClientConnRequest::respond(
                &mut tx,
                ConnStatusCode::UNSUPPORTED_COMMAND,
                &Default::default(),
            )
            .await?;
            Err(anyhow::anyhow!("Unsupported socks command"))
        }
    }
}

pub async fn run_client(
    bind_addr: &str,
    upstream_host: &str,
    upstream_port: u16,
) -> anyhow::Result<()> {
    log::info!("Start client at {}", bind_addr);
    let listener = TcpListener::bind(bind_addr).await?;
    let upstream_addr = format!("{upstream_host}:{upstream_port}");

    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!("Accepted client from: {addr}");

        let upstream_host = upstream_host.to_string();
        let upstream_addr = upstream_addr.clone();
        spawn(async move {
            if let Err(e) = serve_client(sock, &upstream_host, &upstream_addr).await {
                log::error!("Error serving client {addr}: {e}");
            }
            log::info!("Client {addr} disconnected");
        });
    }
}
