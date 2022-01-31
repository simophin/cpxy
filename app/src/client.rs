use bytes::Bytes;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::config::*;
use crate::handshake::Handshaker;
use crate::io::TcpStream;
use crate::proxy::protocol::{ProxyRequest, ProxyResult};
use crate::proxy::request_proxy_upstream;
use crate::udp_relay;
use crate::utils::{copy_duplex, RWBuffer};
use futures_lite::future::race;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use smol::net::TcpListener;
use smol::{spawn, Task};

pub async fn run_client(listener: TcpListener, config: Arc<ClientConfig>) -> anyhow::Result<()> {
    let clients: Arc<Mutex<HashMap<SocketAddr, Task<anyhow::Result<()>>>>> = Default::default();
    let last_visit = LastVisitMap::default();

    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!("Accepted client from: {addr}");

        if let Ok(mut m) = clients.lock() {
            let task: Task<anyhow::Result<()>> = {
                let addr = addr.clone();
                let clients = clients.clone();
                let config = config.clone();
                let last_visit = last_visit.clone();
                let sock = TcpStream::from(sock);
                spawn(async move {
                    if let Err(e) = serve_proxy_client(sock.is_v4(), sock, config, last_visit).await
                    {
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

async fn prepare_direct_tcp(
    req: &ProxyRequest,
) -> anyhow::Result<(
    SocketAddr,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
)> {
    let (dst, init_req) = match req {
        ProxyRequest::TCP { dst } => (dst, Bytes::new()),
        ProxyRequest::Http { dst, request } => (dst, request.clone()),
        ProxyRequest::UDP => unreachable!("Unexpected request type for direct tcp: {req:}"),
    };

    let mut stream = TcpStream::connect(&dst).await?;
    if !init_req.is_empty() {
        stream.write_all(init_req.as_ref()).await?;
    }
    Ok((stream.local_addr()?, stream))
}

async fn drain_socks(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 24];
    while socks.read(buf.as_mut_slice()).await? > 0 {}
    Ok(())
}

async fn serve_proxy_client(
    is_v4: bool,
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    config: Arc<ClientConfig>,
    last_visit: LastVisitMap,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::default();
    let (handshaker, req) = Handshaker::start(&mut socks, &mut buf).await?;
    log::info!("Requesting to proxy {req:?}");

    match &req {
        ProxyRequest::TCP { dst } | ProxyRequest::Http { dst, .. } => {
            if let Some((name, config)) = config.find_best_upstream(last_visit, &dst) {
                log::info!("Using upstream {name} for {dst}");
                match request_proxy_upstream(&config, &req).await {
                    Ok((ProxyResult::Granted { bound_address }, upstream)) => {
                        handshaker.respond_ok(&mut socks, bound_address).await?;
                        copy_duplex(upstream, socks).await
                    }
                    Ok((result, _)) => {
                        handshaker.respond_err(&mut socks).await?;
                        Err(result.into())
                    }
                    Err(e) => {
                        handshaker.respond_err(&mut socks).await?;
                        Err(e.into())
                    }
                }
            } else {
                log::info!("Connecting directly to {dst}");
                match prepare_direct_tcp(&req).await {
                    Ok((bound_address, upstream)) => {
                        handshaker.respond_ok(&mut socks, bound_address).await?;
                        copy_duplex(upstream, socks).await
                    }
                    Err(e) => {
                        handshaker.respond_err(&mut socks).await?;
                        Err(e.into())
                    }
                }
            }
        }

        ProxyRequest::UDP => match udp_relay::Relay::new(config, last_visit, is_v4).await {
            Ok((r, a)) => {
                handshaker.respond_ok(&mut socks, a).await?;
                race(r.run(), drain_socks(socks)).await
            }
            Err(e) => {
                handshaker.respond_err(&mut socks).await?;
                Err(e.into())
            }
        },
    }
}
