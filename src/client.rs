use anyhow::anyhow;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::cipher::strategy::EncryptionStrategy;
use crate::geoip::{choose_ip_addr, CountryCode};
use crate::handshake::Handshaker;
use crate::io::{TcpStream, UdpSocket};
use crate::proxy::protocol::{ProxyRequest, ProxyRequestType as rt, ProxyResult};
use crate::socks5::Address;
use crate::utils::{
    copy_duplex, read_json_lengthed_async, write_json_lengthed, HttpRequest, RWBuffer,
};
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};
use smol::net::TcpListener;
use smol::{spawn, Task};
use smol_timeout::TimeoutExt;

pub struct ClientConfig {
    pub upstream: Address,
    pub upstream_timeout: Duration,
    pub socks5_udp_host: String,
    pub reject_countries: Vec<CountryCode>,
    pub accept_countries: Vec<CountryCode>,
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
    write_json_lengthed(&mut header, req);

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

    Ok((read_json_lengthed_async(&mut upstream).await?, upstream))
}

async fn prepare_direct_tcp(
    config: &ClientConfig,
    resolved: Vec<(IpAddr, CountryCode)>,
    original: &Address,
) -> anyhow::Result<(TcpStream, SocketAddr)> {
    let resolved: Vec<SocketAddr> = choose_ip_addr(
        resolved.iter(),
        config.accept_countries.as_slice(),
        config.reject_countries.as_slice(),
    )
    .map(|(a, _)| SocketAddr::new(a.clone(), original.get_port()))
    .collect();

    let upstream = if resolved.is_empty() {
        TcpStream::connect(original).await?
    } else {
        TcpStream::connect_raw(resolved.as_slice()).await?
    };

    let bound_addr = upstream.local_addr()?;
    Ok((upstream, bound_addr))
}

async fn prepare_direct_http(
    config: &ClientConfig,
    resolved: Vec<(IpAddr, CountryCode)>,
    original_addr: &Address,
    headers: &[u8],
) -> anyhow::Result<(TcpStream, SocketAddr)> {
    let (mut socket, addr) = prepare_direct_tcp(config, resolved, original_addr).await?;
    socket.write_all(headers).await?;
    Ok((socket, addr))
}

async fn serve_proxy_client(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    config: Arc<ClientConfig>,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::default();
    let (handshaker, req) = Handshaker::start(&mut socks, &mut buf).await?;
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
            log::info!("Connecting to udp://{addr} directly");
            todo!()
        }
        (Err(e), _) => {
            handshaker.respond_err(&mut socks).await?;
            return Err(e.into());
        }
    }
}
