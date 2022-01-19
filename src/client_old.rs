use crate::http;
use crate::parse::ParseError;
use anyhow::anyhow;
use httparse::{Request, EMPTY_HEADER};
use std::borrow::Cow;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::{split, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::select;
use tokio::task::spawn;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use crate::socks5::{
    Address, ClientConnRequest, ClientGreeting, Command, ConnStatusCode, AUTH_NOT_ACCEPTED,
    AUTH_NO_PASSWORD,
};
use crate::udp::{copy_frame_to_socks5_udp, copy_socks5_udp_to_frame};
use crate::utils::{copy_io, copy_io_with_buf, RWBuffer};

async fn negotiate_with_server(
    cc: Arc<rustls::ClientConfig>,
    upstream_host: &str,
    upstream_addr: &str,
    protocol: &str,
    address: &Address,
) -> anyhow::Result<(http::ProxyResponse, TlsStream<TcpStream>)> {
    log::info!("Connecting to {}", upstream_addr);
    let upstream = TcpStream::connect(upstream_addr).await?;
    let mut upstream = TlsConnector::from(cc)
        .connect(upstream_host.try_into()?, upstream)
        .await?;
    log::debug!("Connected to {}", upstream_addr);

    http::ProxyRequest::write(protocol, address, upstream_addr, &mut upstream).await?;

    let mut buf = RWBuffer::with_capacity(4096);

    loop {
        match upstream.read(buf.write_buf()).await? {
            0 => return Err(anyhow::anyhow!("Unexpected EOF")),
            v => buf.advance_write(v),
        };

        if let Some(v) = http::ProxyResponse::parse(buf.read_buf())? {
            return Ok((v, upstream));
        }
    }
}

async fn serve_socks5_client_tcp(
    mut sock: TcpStream,
    buf: RWBuffer,
    cc: Arc<rustls::ClientConfig>,
    upstream_host: &str,
    upstream_addr: &str,
    address: Address,
) -> anyhow::Result<()> {
    let (bound, upstream) =
        match negotiate_with_server(cc, &upstream_host, &upstream_addr, "tcp", &address).await {
            Ok(v) => v,
            Err(e) => {
                ClientConnRequest::respond(&mut sock, ConnStatusCode::FAILED, &Default::default())
                    .await?;
                return Err(e.into());
            }
        };

    ClientConnRequest::respond(
        &mut sock,
        ConnStatusCode::GRANTED,
        &Address::IP(bound.bound_address),
    )
    .await?;

    let (rx, tx) = split(sock);
    let (upstream_rx, upstream_tx) = split(upstream);
    select! {
        r1 = copy_io_with_buf(rx, upstream_tx, buf) => r1,
        r2 = copy_io(upstream_rx, tx) => r2,
    }
}

async fn prepare_client_udp(
    cc: Arc<rustls::ClientConfig>,
    upstream_host: &str,
    upstream_addr: &str,
    address: &Address,
) -> anyhow::Result<(UdpSocket, TlsStream<TcpStream>, SocketAddr)> {
    let (_, upstream) =
        negotiate_with_server(cc, &upstream_host, &upstream_addr, "udp", address).await?;

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let local_udp_addr = socket.local_addr()?;
    Ok((socket, upstream, local_udp_addr))
}

async fn serve_client_udp(
    mut sock: TcpStream,
    buf: RWBuffer,
    cc: Arc<rustls::ClientConfig>,
    upstream_host: &str,
    upstream_addr: &str,
    address: Address,
) -> anyhow::Result<()> {
    let (udp_socket, upstream, local_udp_addr) =
        match prepare_client_udp(cc, upstream_host, upstream_addr, &address).await {
            Ok(v) => v,
            Err(e) => {
                ClientConnRequest::respond(&mut sock, ConnStatusCode::FAILED, &Default::default())
                    .await?;
                return Err(e);
            }
        };

    ClientConnRequest::respond(
        &mut sock,
        ConnStatusCode::GRANTED,
        &Address::IP(local_udp_addr),
    )
    .await?;

    let (mut upstream_rx, mut upstream_tx) = split(upstream);

    select! {
        r1 = copy_socks5_udp_to_frame(&udp_socket, &address, &mut upstream_tx, buf) => r1,
        r2 = copy_frame_to_socks5_udp(&mut upstream_rx, &address, &udp_socket) => r2,
    }
}

async fn serve_client_socks5(
    mut sock: TcpStream,
    mut buf: RWBuffer,
    cc: Arc<rustls::ClientConfig>,
    upstream_host: &str,
    upstream_addr: &str,
) -> anyhow::Result<()> {
    loop {
        match ClientConnRequest::parse(buf.read_buf()) {
            Ok(Some((offset, req))) if req.cmd == Command::CONNECT_TCP => {
                let address = req.address;
                buf.advance_read(offset);
                return serve_socks5_client_tcp(
                    sock,
                    buf,
                    cc,
                    upstream_host,
                    upstream_addr,
                    address,
                )
                .await;
            }
            Ok(Some((_, req))) if req.cmd == Command::BIND_UDP => {
                return serve_client_udp(sock, buf, cc, upstream_host, upstream_addr, req.address)
                    .await
            }
            Ok(Some((_, req))) => {
                ClientConnRequest::respond(
                    &mut sock,
                    ConnStatusCode::UNSUPPORTED_COMMAND,
                    &Default::default(),
                )
                .await?;
                return Err(ParseError::unexpected("command", req.cmd, "1 or 3").into());
            }
            Err(e) => return Err(e.into()),
            _ => {}
        }

        match sock.read(buf.write_buf()).await? {
            0 => return Err(anyhow!("Unexpected EOF")),
            v => buf.advance_write(v),
        }
    }
}

async fn serve_http_with_upstream(
    mut sock: TcpStream,
    mut buf: RWBuffer,
    cc: Arc<rustls::ClientConfig>,
    upstream_host: &str,
    upstream_addr: &str,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
) -> anyhow::Result<()> {
    unimplemented!()
}

async fn serve_http_client(
    mut sock: TcpStream,
    req: http::HttpRequest,
    mut buf: RWBuffer,
    cc: Arc<rustls::ClientConfig>,
    upstream_host: &str,
    upstream_addr: &str,
) -> anyhow::Result<()> {
    let http::HttpRequest {
        mut method,
        path,
        headers,
    } = req;
    match (
        method.map(|mut x| {
            x.make_ascii_uppercase();
            x
        }),
        path,
    ) {
        (Some(method), Some(path)) if method == "CONNECT" => {
            let addr = match Address::from_str(&path) {
                Ok(a) => a,
                Err(e) => {
                    sock.write_all(b"HTTP/1.1 400 Invalid address").await?;
                    return Err(e);
                }
            };
            let (resp, upstream) =
                match negotiate_with_server(cc, upstream_host, upstream_addr, "tcp", &addr).await {
                    Ok(v) => v,
                    Err(e) => {
                        sock.write_all(b"HTTP/1.1 500").await?;
                        return Err(e);
                    }
                };

            Ok(())
        }
        _ => {
            sock.write_all(b"HTTP/1.1 401 unsupported").await?;
            Ok(())
        }
    }
}

async fn serve_client(
    mut sock: TcpStream,
    cc: Arc<rustls::ClientConfig>,
    upstream_host: &str,
    upstream_addr: &str,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::with_capacity(65536);

    let http_request = loop {
        match sock.read(buf.write_buf()).await? {
            0 => return Err(anyhow!("Unexpected EOF")),
            v => buf.advance_write(v),
        };

        let mut headers = [EMPTY_HEADER; 20];
        let mut req = httparse::Request::new(&mut headers);

        match (
            ClientGreeting::parse(buf.read_buf()),
            req.parse(buf.read_buf()),
        ) {
            (Err(e1), Err(e2)) => {
                return Err(ParseError::unexpected(
                    "greeting message",
                    format!("{e1} and {e2}"),
                    "Valid socks5 or http",
                )
                .into())
            }
            (Ok(Some((offset, greeting))), _) => {
                if !greeting.auths.contains(&AUTH_NO_PASSWORD) {
                    ClientGreeting::respond(AUTH_NOT_ACCEPTED, &mut sock).await?;
                    return Err(ParseError::unexpected(
                        "SOCK5 AUTH",
                        greeting.auths.to_vec(),
                        "0x00",
                    )
                    .into());
                }

                ClientGreeting::respond(AUTH_NO_PASSWORD, &mut sock).await?;
                buf.advance_read(offset);
                break None;
            }
            (_, Ok(httparse::Status::Complete(offset))) => {
                let result = req.into();
                buf.advance_read(offset);
                break Some(result);
            }
            _ => continue,
        }
    };

    match http_request {
        Some(req) => serve_http_client(sock, req, buf, cc, upstream_host, upstream_addr).await,
        None => serve_client_socks5(sock, buf, cc, upstream_host, upstream_addr).await,
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
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
        rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(
            ta.subject,
            ta.spki,
            ta.name_constraints,
        )
    }));

    let config = Arc::new(
        rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    );

    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!("Accepted client from: {addr}");

        let upstream_host = upstream_host.to_string();
        let upstream_addr = upstream_addr.clone();
        let config = config.clone();
        spawn(async move {
            if let Err(e) = serve_client(sock, config, &upstream_host, &upstream_addr).await {
                log::error!("Error serving client {addr}: {e}");
            }
            log::info!("Client {addr} disconnected");
        });
    }
}
