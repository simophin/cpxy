use anyhow::anyhow;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cipher::strategy::EncryptionStrategy;
use crate::geoip::CountryCode;
use crate::handshake::Handshaker;
use crate::io::{TcpStream, UdpSocket};
use crate::proxy::protocol::{IPPolicy, ProxyRequest, ProxyRequestType as rt, ProxyResult};
use crate::socks5::{serve_socks5_udp_direct_relay, serve_socks5_udp_stream_relay, Address};
use crate::utils::{copy_duplex, read_bincode_lengthed_async, write_bincode_lengthed, RWBuffer};
use futures_lite::future::race;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use smol::net::TcpListener;
use smol::{spawn, Task};
use smol_timeout::TimeoutExt;

pub struct ClientConfig {
    pub upstream: Address,
    pub upstream_timeout: Duration,
    pub upstream_policy: IPPolicy,
    pub socks5_udp_host: String,
    pub local_policy: IPPolicy,
}

pub async fn run_client(listener: TcpListener, config: Arc<ClientConfig>) -> anyhow::Result<()> {
    let clients: Arc<Mutex<HashMap<SocketAddr, Task<anyhow::Result<()>>>>> = Default::default();

    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!("Accepted client from: {addr}");

        if let Ok(mut m) = clients.lock() {
            let task: Task<anyhow::Result<()>> = {
                let addr = addr.clone();
                let clients = clients.clone();
                let config = config.clone();
                spawn(async move {
                    if let Err(e) = serve_proxy_client(sock, config).await {
                        log::error!("Error serving client {addr}: {e}");
                    }
                    log::info!("Client {addr} disconnected");
                    if let Ok(mut m) = clients.lock() {
                        m.remove(&addr);
                    }

                    Ok(())
                })
            };
            m.insert(addr, task);
        }
    }
}

async fn prepare_upstream(
    c: &ClientConfig,
    req: &ProxyRequest,
) -> anyhow::Result<(
    ProxyResult,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
)> {
    let send_enc = EncryptionStrategy::pick_send(&req);
    let receive_enc = EncryptionStrategy::pick_receive(&req);

    log::debug!("EncryptionStrategy(send = {send_enc:?}, receive = {receive_enc})");

    let mut header = Vec::new();
    write_bincode_lengthed(&mut header, req)?;

    let upstream = TcpStream::connect(&c.upstream)
        .timeout(c.upstream_timeout)
        .await
        .ok_or_else(|| anyhow!("Timeout connecting to {}", c.upstream))??;

    let mut upstream = super::cipher::client::connect(
        upstream,
        c.upstream.get_host().as_ref(),
        send_enc,
        receive_enc,
        header,
    )
    .await?;

    Ok((read_bincode_lengthed_async(&mut upstream).await?, upstream))
}

fn choose_resolved_addresses(
    config: &ClientConfig,
    resolved: Vec<(IpAddr, Option<CountryCode>)>,
    original: &Address,
) -> Vec<SocketAddr> {
    let mut addrs: Vec<(SocketAddr, Option<CountryCode>)> = resolved
        .into_iter()
        .filter(|(ip, c)| config.local_policy.should_keep(ip, *c))
        .map(|(ip, c)| (SocketAddr::new(ip, original.get_port()), c))
        .collect();

    config.local_policy.sort_by_preferences(&mut addrs);
    addrs.into_iter().map(|(addr, _)| addr).collect()
}

async fn prepare_direct_tcp(
    config: &ClientConfig,
    resolved: Vec<(IpAddr, Option<CountryCode>)>,
    original: &Address,
) -> anyhow::Result<(TcpStream, SocketAddr)> {
    let upstream = match choose_resolved_addresses(config, resolved, original) {
        v if !v.is_empty() => TcpStream::connect_raw(v.as_slice()).await?,
        _ => TcpStream::connect(original).await?,
    };

    let bound_addr = upstream.local_addr()?;
    Ok((upstream, bound_addr))
}

async fn prepare_direct_http(
    config: &ClientConfig,
    resolved: Vec<(IpAddr, Option<CountryCode>)>,
    original_addr: &Address,
    headers: &[u8],
) -> anyhow::Result<(TcpStream, SocketAddr)> {
    let (mut socket, addr) = prepare_direct_tcp(config, resolved, original_addr).await?;
    socket.write_all(headers).await?;
    Ok((socket, addr))
}

async fn prepare_direct_udp(
    config: &ClientConfig,
    resolved: Vec<(IpAddr, Option<CountryCode>)>,
    original_addr: Option<&Address>,
    is_v4: bool,
) -> anyhow::Result<(UdpSocket, UdpSocket, SocketAddr)> {
    match original_addr.map(|addr| choose_resolved_addresses(config, resolved, addr)) {
        Some(addr) if addr.is_empty() => {
            return Err(anyhow!("Unable to resolve address due to IPPolicy"))
        }
        _ => {}
    };

    let upstream = UdpSocket::bind(is_v4).await?;
    let socks = UdpSocket::bind(is_v4).await?;
    let addr = socks.local_addr()?;
    Ok((upstream, socks, addr))
}

async fn prepare_relay_udp(is_v4: bool) -> anyhow::Result<(UdpSocket, SocketAddr)> {
    let socks = UdpSocket::bind(is_v4).await?;
    let addr = socks.local_addr()?;
    Ok((socks, addr))
}

async fn drain_socks(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 24];
    while socks.read(buf.as_mut_slice()).await? > 0 {}
    Ok(())
}

async fn serve_proxy_client(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    config: Arc<ClientConfig>,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::default();
    let (handshaker, req) =
        Handshaker::start(&mut socks, &mut buf, config.local_policy.clone()).await?;
    log::info!("Requesting to proxy {req:?}");

    match (prepare_upstream(config.as_ref(), &req).await, &req.t) {
        (
            Ok((ProxyResult::Granted { bound_address }, upstream)),
            rt::SocksTCP(_) | rt::Http(_, _),
        ) => {
            log::info!("Proxying {req:?}");
            handshaker.respond_ok(&mut socks, bound_address).await?;
            copy_duplex(upstream, socks).await
        }
        (Ok((ProxyResult::Granted { .. }, upstream)), rt::SocksUDP(_)) => {
            log::info!("Proxying {req:?}");
            let socket = match prepare_relay_udp(true).await {
                Ok((v, addr)) => {
                    handshaker.respond_ok(&mut socks, addr).await?;
                    v
                }
                Err(e) => {
                    handshaker.respond_err(&mut socks).await?;
                    return Err(e.into());
                }
            };
            race(
                drain_socks(socks),
                serve_socks5_udp_stream_relay(socket, upstream),
            )
            .await
        }
        (Ok((ProxyResult::ErrHostRejected { resolved }, ..)), rt::SocksTCP(addr)) => {
            log::info!("Connecting to tcp://{addr} directly");
            match prepare_direct_tcp(config.as_ref(), resolved, addr).await {
                Ok((s, addr)) => {
                    handshaker.respond_ok(&mut socks, addr).await?;
                    copy_duplex(s, socks).await
                }
                Err(e) => {
                    handshaker.respond_err(&mut socks).await?;
                    return Err(e.into());
                }
            }
        }
        (Ok((ProxyResult::ErrHostRejected { resolved }, ..)), rt::Http(addr, headers)) => {
            log::info!("Connecting to http://{addr} directly");
            match prepare_direct_http(config.as_ref(), resolved, addr, headers).await {
                Ok((s, addr)) => {
                    handshaker.respond_ok(&mut socks, addr).await?;
                    copy_duplex(s, socks).await
                }
                Err(e) => {
                    handshaker.respond_err(&mut socks).await?;
                    return Err(e.into());
                }
            }
        }
        (Ok((ProxyResult::ErrHostRejected { resolved }, ..)), rt::SocksUDP(addr)) => {
            log::info!("Connecting to udp://{addr:?} directly");
            let (upstream_sock, socks5_sock) =
                match prepare_direct_udp(config.as_ref(), resolved, addr.as_ref(), true).await {
                    Ok((s1, s2, bound_addr)) => {
                        handshaker.respond_ok(&mut socks, bound_addr).await?;
                        (s2, s1)
                    }
                    Err(e) => {
                        handshaker.respond_err(&mut socks).await?;
                        return Err(e);
                    }
                };
            race(
                serve_socks5_udp_direct_relay(socks5_sock, upstream_sock),
                drain_socks(socks),
            )
            .await
        }
        (Ok((v, _)), _) => {
            handshaker.respond_err(&mut socks).await?;
            return Err(v.into());
        }
        (Err(e), _) => {
            handshaker.respond_err(&mut socks).await?;
            return Err(e.into());
        }
    }
}
