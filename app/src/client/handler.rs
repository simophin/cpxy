use std::sync::Arc;

use anyhow::Context;
use futures_lite::{Stream, StreamExt};
use futures_util::{select, FutureExt};
use smol::{spawn, Executor, Task};

use crate::{
    buf::RWBuffer,
    config::ClientConfig,
    handshake::{HandshakeRequest as HR, Handshaker},
    io::{TcpListener, TcpStream},
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
        current_tasks.clear();

        let proxy_listener = match TcpListener::bind(&Address::IP(config.socks5_address)).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error listening for TCP proxy: {e:?}");
                continue;
            }
        };

        // let socket = match UdpSocket::bind_raw(&config.socks5_address).await {
        //     Ok(v) => v,
        //     Err(e) => {
        //         log::error!("Error listening for UDP tproxy: {e:?}");
        //         continue;
        //     }
        // };

        // match start_udp_client_with(socket, config.clone(), stats.clone()).await {
        //     Ok(task) => current_tasks.push(task),
        //     Err(e) => {
        //         log::error!("Error starting UDP client: {e:?}");
        //         continue;
        //     }
        // };

        {
            let config = config.clone();
            current_tasks.push(spawn(run_proxy_with(proxy_listener, config, stats)));
        }

        log::info!("Proxy server listening on {}", config.socks5_address);
    }
}

pub async fn run_proxy_with(
    proxy_listener: TcpListener,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<()> {
    let executor = Executor::new();
    loop {
        let (sock, addr) = select! {
            v1 = proxy_listener.accept().fuse() => v1.context("Listening for SOCKS5/SOCKS4/HTTP/TPROXY connection")?,
            _ = executor.tick().fuse() => {
                let mut tick_num = 10;
                while executor.try_tick() && tick_num >= 0 {
                    tick_num -= 1;
                }
                continue;
            },
        };

        let config = config.clone();
        let stats = stats.clone();
        executor
            .spawn(async move {
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
    let mut buf = RWBuffer::new(128, 65536);
    let transparent_addr = socks.get_original_dst();
    let (hs, req) = Handshaker::start(&mut socks, transparent_addr, &mut buf)
        .await
        .context("Handshaking")?;
    log::info!("Requesting to proxy {req:?}");

    match req {
        HR::TCP { dst } => serve_tcp_proxy_conn(dst, &config, &stats, socks, hs).await,
        HR::HTTP { dst, https, req } => {
            serve_http_proxy_conn(dst, https, req, &config, &stats, socks, hs).await
        }

        HR::UDP { dst } => serve_udp_proxy_conn(&config, &stats, socks.is_v4(), socks, hs).await,
    }
}
