use std::{net::SocketAddr, os::unix::prelude::AsRawFd, sync::Arc};

use anyhow::{bail, Context};
use nix::sys::socket::{setsockopt, sockopt::IpTransparent, SockAddr};
use smol::spawn;

use crate::{
    client::ClientStatistics,
    config::ClientConfig,
    io::{TcpListener, TcpStream},
    proxy::{
        protocol::{ProxyRequest, ProxyResult},
        request_proxy_upstream,
    },
    socks5::Address,
    utils::copy_duplex,
};

pub async fn serve_transparent_proxy_client(
    listener: TcpListener,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    setsockopt(listener.as_raw_fd(), IpTransparent, &true)?;
    loop {
        let (socket, addr) = listener.accept().await?;
        log::info!("Accepted client from {addr}");
        let target = match nix::sys::socket::getsockname(socket.as_raw_fd())
            .context("Getting the sockname of incoming socket")?
        {
            SockAddr::Inet(addr) => addr.to_std(),
            _ => bail!("Unknown socket address type"),
        };

        let config = config.clone();
        let stats = stats.clone();
        spawn(async move {
            if let Err(e) = serve_client(socket, target, config, stats).await {
                log::error!("Error serving client {addr}: {e:?}");
            }
            log::info!("Client {addr} disconnected");
        })
        .detach();
    }
}

async fn serve_client(
    socket: TcpStream,
    target: SocketAddr,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    let target = Address::IP(target);
    let mut upstreams = config.find_best_upstream(stats.as_ref(), &target);
    if !upstreams.is_empty() {
        let proxy_request = ProxyRequest::TCP { dst: target };
        while let Some((name, config)) = upstreams.pop() {
            log::debug!("Trying upstream {name}");
            match request_proxy_upstream(config, &proxy_request).await? {
                (ProxyResult::Granted { .. }, upstream, delay) => {
                    stats.update_upstream(name, delay);
                    let (tx, rx) = stats
                        .upstreams
                        .get(name)
                        .map(|s| (Some(s.tx.clone()), Some(s.rx.clone())))
                        .unwrap_or((None, None));
                    return copy_duplex(socket, upstream, tx, rx).await;
                }
                (r, _, _) => {
                    log::error!("Error proxying through upstream {name}: {r:?}");
                    continue;
                }
            }
        }

        bail!("No upstream is available to serve this request")
    } else if config.allow_direct(&target) {
        log::debug!("Connect directly to {target}");
        let upstream = TcpStream::connect(&target).await?;
        copy_duplex(socket, upstream, None, None).await
    } else {
        bail!("No way to connect to target: {target}")
    }
}
