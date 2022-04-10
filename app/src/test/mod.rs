use std::{net::SocketAddr, sync::Arc, time::Duration, vec};

use anyhow::bail;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use futures_util::join;
use maplit::hashmap;
use smol::{block_on, spawn, Task};

mod http;
mod tcp_socks4;
mod tcp_socks5;
mod udp;

use crate::{
    buf::RWBuffer,
    client::{run_proxy_with, ClientStatistics},
    config::{ClientConfig, UpstreamConfig},
    fetch::fetch_http_with_proxy,
    io::{TcpListener, TcpStream, UdpSocket},
    server::run_server,
    socks5::Address,
};

pub async fn duplex(
    _: usize,
) -> (
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) {
    let listener = TcpListener::bind(&"127.0.0.1:0".parse().unwrap())
        .await
        .expect("To listen");
    let addr = Address::IP(listener.local_addr().expect("To have local addr"));

    let (client, server) = join!(TcpStream::connect(&addr), listener.accept());
    let client = client.expect("To connect");
    let (server, _) = server.expect("To accept");

    (client, server)
}

pub async fn create_http_server() -> (TcpListener, String) {
    let listener = TcpListener::bind(&"127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();

    let addr = listener.local_addr().unwrap();
    (listener, format!("http://{addr}"))
}

pub async fn echo_tcp_server() -> (Task<()>, SocketAddr) {
    let socket = TcpListener::bind(&Default::default()).await.unwrap();
    let addr = socket.local_addr().unwrap();
    (
        spawn(async move {
            loop {
                let (mut socket, _) = socket.accept().await.unwrap();
                spawn(async move {
                    let mut buf = vec![0; 4096];
                    loop {
                        match socket.read(buf.as_mut_slice()).await.unwrap() {
                            0 => return,
                            v => socket.write_all(&buf[..v]).await.unwrap(),
                        }
                    }
                })
                .detach();
            }
        }),
        addr,
    )
}

pub async fn echo_udp_server() -> (Task<()>, SocketAddr) {
    let socket = UdpSocket::bind(true).await.unwrap();
    let addr = socket.local_addr().unwrap();
    (
        spawn(async move {
            let mut buf = vec![0; 65536];
            loop {
                let (len, addr) = socket.recv_from(buf.as_mut_slice()).await.unwrap();
                socket.send_to(&buf[..len], addr).await.unwrap();
            }
        }),
        addr,
    )
}

pub async fn run_test_client(upstream_address: SocketAddr) -> (Task<()>, SocketAddr) {
    let listener = TcpListener::bind(&Default::default()).await.unwrap();
    let addr = listener.local_addr().unwrap();

    (
        {
            let socks5_address = addr.clone();
            spawn(async move {
                let config = ClientConfig {
                    socks5_address,
                    direct_accept: Default::default(),
                    direct_reject: Default::default(),
                    upstreams: hashmap! {
                        String::from("echo") => UpstreamConfig {
                            address: Address::IP(upstream_address),
                            tls: false,
                            accept: Default::default(),
                            reject: Default::default(),
                            priority: 0,
                            enabled: true
                        }
                    },
                    socks5_udp_host: "0.0.0.0".parse().unwrap(),
                };
                let stats = ClientStatistics::new(&config);

                run_proxy_with(listener, Arc::new(config), Arc::new(stats))
                    .await
                    .unwrap();
            })
        },
        addr,
    )
}

pub async fn run_test_server() -> (Task<()>, SocketAddr) {
    let listener = TcpListener::bind(&Default::default()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    (
        spawn(async move { run_server(listener).await.unwrap() }),
        addr,
    )
}

async fn read_exact(r: &mut (impl AsyncRead + Unpin), n: usize) -> anyhow::Result<Vec<u8>> {
    let mut buf = vec![0; n];
    r.read_exact(buf.as_mut_slice()).await?;
    Ok(buf)
}

async fn parse_address(
    r: &mut (impl AsyncRead + Unpin),
) -> anyhow::Result<(Address<'static>, RWBuffer)> {
    let mut buf = RWBuffer::new(128, 520);
    loop {
        match r.read(buf.write_buf()).await? {
            0 => bail!("Unexpected EOF"),
            v => buf.advance_write(v),
        };

        match Address::parse(buf.read_buf())? {
            None => continue,
            Some((offset, addr)) => {
                let addr = addr.into_owned();
                buf.advance_read(offset);
                return Ok((addr, buf));
            }
        }
    }
}

async fn send_socks5_request(
    socks: &mut (impl AsyncRead + AsyncWrite + Unpin + Send + Sync),
    target: &Address<'_>,
    is_udp: bool,
) -> anyhow::Result<Address<'static>> {
    // Send greetings
    socks.write_all(&[0x5, 0x1, 0x0]).await?;

    // Expect response
    let res = read_exact(socks, 2).await?;
    assert_eq!(res[0], 0x5);
    assert_eq!(res[1], 0x0);

    // Send connection request
    socks
        .write_all(&[
            0x5,
            if is_udp { 0x3 } else { 0x1 },
            0x00, // RSV
        ])
        .await?;
    target.write(socks).await?;

    // Recv conn response
    let res = read_exact(socks, 3).await?;
    let (bound, buf) = parse_address(socks).await?;
    assert_eq!(buf.remaining_read(), 0);
    assert_eq!(res[0], 0x5);
    let code = res[1];
    if code != 0 {
        bail!("Connection refused with code = {code}");
    }
    Ok(bound)
}

const TIMEOUT: Duration = Duration::from_secs(3);
