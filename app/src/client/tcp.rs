use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{anyhow, Context};
use futures_lite::{AsyncRead, AsyncWrite};
use futures_util::{select, FutureExt};
use smol::Executor;
use smol_timeout::TimeoutExt;

use crate::{
    config::ClientConfig,
    io::{TcpListener, TcpStream},
    socks5::Address,
    stream::AsyncReadWrite,
};

use super::ClientStatistics;

pub async fn run_tcp_client_with(
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

                if let Err(e) = super::serve_proxy_client(sock, config, stats).await {
                    log::error!("Error serving client {addr}: {e:?}");
                }

                log::info!("Client {addr} disconnected");
            })
            .detach();
    }
}

pub async fn prepare_direct_tcp(
    dst: Address<'_>,
) -> anyhow::Result<(
    Option<SocketAddr>,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
)> {
    let stream = TcpStream::connect(&dst)
        .timeout(Duration::from_secs(2))
        .await
        .ok_or_else(|| anyhow!("Timeout connecting to {dst}"))??;
    Ok((stream.local_addr().ok(), AsyncReadWrite::new(stream)))
}
