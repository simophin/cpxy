use std::sync::Arc;

use crate::{io::bind_tcp, iptables as ipt};
use anyhow::Context;
use async_shutdown::Shutdown;
use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio_stream::{Stream, StreamExt};

use crate::handshaker::Handshaker;
use crate::{config::ClientConfig, socks5::Address};

use super::{tcp::serve_tcp_proxy_conn, ClientStatistics};

pub async fn run_client(
    shutdown: Shutdown,
    mut config_stream: impl Stream<Item = Arc<ClientConfig>> + Unpin,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    let mut last_shutdown: Option<Shutdown> = None;
    loop {
        log::debug!("Listening for next config");
        let next_config = shutdown.wrap_cancel(config_stream.next()).await;
        if let Some(s) = last_shutdown.take() {
            s.shutdown();
            s.wait_shutdown_complete().await;
        }

        let config = match next_config {
            Some(Some(v)) => v,
            Some(None) | None => {
                log::info!("Socks5 server stopped");
                return Ok(());
            }
        };

        log::debug!("Using configuration {config:?}");
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

        let shutdown = Shutdown::new();

        spawn(shutdown.wrap_cancel(run_proxy_with(
            shutdown.clone(),
            proxy_listener,
            config.clone(),
            stats.clone(),
        )));

        last_shutdown.replace(shutdown);

        log::info!("Proxy server listening on {}", config.socks5_address);
    }
}

pub async fn run_proxy_with(
    shutdown: Shutdown,
    proxy_listener: TcpListener,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    loop {
        let (sock, addr) = proxy_listener
            .accept()
            .await
            .context("Listening for SOCKS5/SOCKS4/HTTP connection")?;

        let config = config.clone();
        let stats = stats.clone();
        let shutdown = shutdown.clone();
        spawn(async move {
            log::info!("Client {addr} connected");
            if let Some(Err(e)) = shutdown
                .wrap_cancel(serve_proxy_conn(sock, config, stats))
                .await
            {
                log::error!("Error serving client {addr}: {e:?}");
            }
            log::info!("Client {addr} disconnected");
        });
    }
}

async fn serve_proxy_conn(
    socks: TcpStream,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    let (hs, req) = Handshaker::start(BufReader::new(socks))
        .await
        .context("Handshaking")?;
    log::info!("Requesting to proxy {req:?}");
    serve_tcp_proxy_conn(req, hs, &config, &stats).await
}
