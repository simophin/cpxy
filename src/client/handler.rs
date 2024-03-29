use std::sync::Arc;

use crate::{
    client::tcp::serve_tcp_tproxy_conn,
    io::{bind_tcp, TcpStreamExt},
    iptables as ipt,
};
use anyhow::Context;
use futures::{Stream, StreamExt};
use smol::{
    net::{TcpListener, TcpStream},
    spawn, Task,
};

use crate::{
    buf::RWBuffer,
    config::ClientConfig,
    handshake::{HandshakeRequest as HR, Handshaker},
    socks5::Address,
};

use super::{
    http::serve_http_proxy_conn, tcp::serve_tcp_proxy_conn, udp::serve_udp_proxy_conn,
    ClientStatistics,
};

pub async fn run_client(
    mut config_stream: impl Stream<Item = (Arc<ClientConfig>, Arc<ClientStatistics>)>
        + Send
        + Sync
        + Unpin,
) -> anyhow::Result<()> {
    let mut current_tasks = Vec::<Task<_>>::with_capacity(2);
    loop {
        log::debug!("Listening for next config");
        let (config, stats) = match config_stream.next().await {
            Some(v) => v,
            None => {
                log::info!("Socks5 server stopped");
                return Ok(());
            }
        };

        log::debug!("Using configuration {config:?}");
        while let Some(task) = current_tasks.pop() {
            task.cancel().await;
        }
        let _ = ipt::clean_up();

        let proxy_listener = match bind_tcp(&Address::IP(config.socks5_address)).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error listening for TCP proxy: {e:?}");
                continue;
            }
        };

        if config.set_router_rules {
            if let Err(e) = ipt::add_rules(
                config.socks5_address.port(),
                config.udp_tproxy_address.map(|v| v.port()),
            ) {
                log::error!("Error setting router rules: {e:?}");
                let _ = ipt::clean_up();
            } else {
                log::info!("Successfully set router rules");
            }
        }

        current_tasks.push(spawn(run_proxy_with(
            proxy_listener,
            config.clone(),
            stats.clone(),
        )));

        // UDP tproxy?
        #[cfg(target_os = "linux")]
        {
            if let Some(addr) = config.udp_tproxy_address {
                match super::transparent::serve_udp_transparent_proxy(
                    addr,
                    config.clone(),
                    stats.clone(),
                )
                .await
                {
                    Ok(task) => current_tasks.push(task),
                    Err(e) => {
                        log::error!("Error serving udp transparent proxy: {e:?}");
                    }
                }
            }
        }

        log::info!("Proxy server listening on {}", config.socks5_address);
    }
}

pub async fn run_proxy_with(
    proxy_listener: TcpListener,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    loop {
        let (sock, addr) = proxy_listener
            .accept()
            .await
            .context("Listening for SOCKS5/SOCKS4/HTTP/TPROXY connection")?;

        let config = config.clone();
        let stats = stats.clone();
        spawn(async move {
            log::info!("Client {addr} connected");
            if let Err(e) = serve_proxy_conn(sock, config, stats).await {
                log::error!("Error serving client {addr}: {e:?}");
            }
            log::info!("Client {addr} disconnected");
        })
        .detach();
    }
}

async fn serve_proxy_conn(
    mut socks: TcpStream,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    if let Some(orig_dst) = socks.get_original_dst() {
        log::info!("Requesting to proxy to {orig_dst} transparently");
        return serve_tcp_tproxy_conn(orig_dst.into(), &config, &stats, socks).await;
    }

    let mut buf = RWBuffer::new_vec_uninitialised(512);
    let (hs, req) = Handshaker::start(&mut socks, &mut buf)
        .await
        .context("Handshaking")?;
    log::info!("Requesting to proxy {req:?}");

    match req {
        HR::TCP { dst } => serve_tcp_proxy_conn(dst, &config, &stats, socks, hs).await,
        HR::HTTP { dst, https, req } => {
            serve_http_proxy_conn(dst, https, req, &config, &stats, socks, hs).await
        }

        HR::UDP { .. } => serve_udp_proxy_conn(&config, &stats, socks.is_v4(), socks, hs).await,
    }
}
