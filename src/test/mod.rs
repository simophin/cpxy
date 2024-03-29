use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
    vec,
};

use crate::{
    config::UpstreamProtocol,
    io::{bind_tcp, bind_udp, connect_tcp},
    protocol::tcpman::{server::run_server, TcpMan},
};
use anyhow::bail;
use async_net::{TcpListener, UdpSocket};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use futures_util::join;
use maplit::hashmap;
use smol::{spawn, Task};

mod http;
mod tcp_socks4;
mod tcp_socks5;
mod udp;

use crate::{
    buf::RWBuffer,
    client::{run_proxy_with, ClientStatistics},
    config::{ClientConfig, UpstreamConfig},
    fetch::fetch_http_with_proxy,
    // server::run_server,
    socks5::Address,
};

#[allow(dead_code)]
pub async fn duplex(
    _: usize,
) -> (
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) {
    let listener = bind_tcp(&"127.0.0.1:0".parse().unwrap())
        .await
        .expect("To listen");
    let addr = Address::IP(listener.local_addr().expect("To have local addr"));

    let (client, server) = join!(connect_tcp(&addr), listener.accept());
    let client = client.expect("To connect");
    let (server, _) = server.expect("To accept");

    (client, server)
}

pub fn set_ip_local(addr: &mut SocketAddr) {
    match addr {
        SocketAddr::V4(a) => a.set_ip(Ipv4Addr::new(127, 0, 0, 1)),
        SocketAddr::V6(a) => a.set_ip("::1".parse().unwrap()),
    }
}

pub fn set_ip_local_address(addr: &mut Address<'_>) {
    match addr {
        Address::IP(a) => set_ip_local(a),
        _ => {}
    }
}

pub async fn create_http_server() -> (TcpListener, String) {
    let listener = bind_tcp(&"127.0.0.1:0".parse().unwrap()).await.unwrap();

    let mut addr = listener.local_addr().unwrap();
    set_ip_local(&mut addr);
    (listener, format!("http://{addr}"))
}

pub async fn create_tcp_server() -> (TcpListener, SocketAddr) {
    let listener = bind_tcp(&"127.0.0.1:0".parse().unwrap()).await.unwrap();

    let mut addr = listener.local_addr().unwrap();
    set_ip_local(&mut addr);
    (listener, addr)
}

pub async fn create_udp_socket() -> (UdpSocket, SocketAddr) {
    let socket = bind_udp(true).await.unwrap();
    let mut addr = socket.local_addr().unwrap();
    set_ip_local(&mut addr);
    (socket, addr)
}

pub async fn echo_tcp_server() -> (Task<()>, SocketAddr) {
    let socket = bind_tcp(&Default::default()).await.unwrap();
    let mut addr = socket.local_addr().unwrap();
    set_ip_local(&mut addr);
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
    let socket = bind_udp(true).await.unwrap();
    let mut addr = socket.local_addr().unwrap();
    addr.set_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
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
    let listener = bind_tcp(&Default::default()).await.unwrap();
    let mut addr = listener.local_addr().unwrap();
    set_ip_local(&mut addr);

    (
        {
            let socks5_address = addr.clone();
            spawn(async move {
                let config = ClientConfig {
                    socks5_address,
                    upstreams: hashmap! {
                        String::from("echo") => UpstreamConfig {
                            protocol: UpstreamProtocol::TcpMan(TcpMan {
                                address: Address::IP(upstream_address),
                                ssl: false,
                                allows_udp: true,
                                credentials: None,
                            }),
                            enabled: true,
                            groups: Default::default(),
                        }
                    },
                    socks5_udp_host: "0.0.0.0".parse().unwrap(),
                    fwmark: None,
                    udp_tproxy_address: None,
                    traffic_rules: Default::default(),
                    set_router_rules: false,
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
    let listener = bind_tcp(&Default::default()).await.unwrap();
    let mut addr = listener.local_addr().unwrap();
    set_ip_local(&mut addr);
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
    let mut buf = RWBuffer::new_vec_uninitialised(128);
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

const TIMEOUT: Duration = Duration::from_secs(2);
