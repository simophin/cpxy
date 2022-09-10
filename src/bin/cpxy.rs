use anyhow::Context;
use async_net::{TcpListener, UdpSocket};
use clap::{Parser, Subcommand};
use cpxy::controller::run_controller;
use cpxy::io::bind_tcp;
use cpxy::protocol::{firetcp, tcpman, udpman};
use cpxy::socks5::Address;
use futures::future::select_all;
use futures::Future;
use smol::{spawn, Task};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

// #[cfg(not(target_env = "msvc"))]
// #[global_allocator]
// static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// SOCKS5 over HTTPs
#[derive(Parser)]
struct Cli {
    #[clap(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    #[clap()]
    Server {
        /// The address to listen on
        #[clap(default_value = "0.0.0.0", long)]
        host: IpAddr,
        /// The TCPMan port to listen on
        #[clap(long)]
        tcpman_port: Option<u16>,
        /// The UDPMan port to listen on
        #[clap(long)]
        udpman_port: Option<u16>,
        #[clap(long)]
        firetcp_port: Option<u16>,
    },

    #[clap()]
    Client {
        #[clap(long)]
        /// Path to the configuration file
        config: String,

        #[clap(default_value = "127.0.0.1", long)]
        controller_host: IpAddr,

        #[clap(default_value_t = 4000, long)]
        controller_port: u16,
    },
}

async fn start_serving_tcp<Fut: Future<Output = anyhow::Result<()>> + Send + Sync + 'static>(
    service_name: &str,
    host: IpAddr,
    port: u16,
    run: impl FnOnce(TcpListener) -> Fut,
) -> anyhow::Result<Task<anyhow::Result<()>>> {
    let addr = SocketAddr::new(host, port);
    let listener = bind_tcp(&Address::IP(addr))
        .await
        .with_context(|| format!("Binding on {addr} for {service_name}"))?;
    log::info!("{service_name} started on {addr}");
    Ok(spawn(run(listener)))
}

async fn start_serving_udp<Fut: Future<Output = anyhow::Result<()>> + Send + Sync + 'static>(
    service_name: &str,
    host: IpAddr,
    port: u16,
    run: impl FnOnce(UdpSocket) -> Fut,
) -> anyhow::Result<Task<anyhow::Result<()>>> {
    let addr = SocketAddr::new(host, port);
    let listener = UdpSocket::bind(addr)
        .await
        .with_context(|| format!("Binding on {addr} for {service_name}"))?;
    log::info!("{service_name} started on {addr}");
    Ok(spawn(run(listener)))
}

fn main() -> anyhow::Result<()> {
    smol::block_on(async move {
        if std::env::var_os("RUST_LOG").is_none() {
            std::env::set_var("RUST_LOG", "info");
        }

        env_logger::init();

        let Cli { cmd } = Cli::parse();
        match cmd {
            Command::Server {
                host,
                tcpman_port,
                udpman_port,
                firetcp_port,
            } => {
                let mut tasks = Vec::<Task<anyhow::Result<()>>>::new();

                if let Some(port) = tcpman_port {
                    tasks.push(
                        start_serving_tcp("tcpman", host, port, tcpman::server::run_server).await?,
                    );
                }

                if let Some(port) = firetcp_port {
                    let password = std::env::var("FIRETCP_PASSWORD")
                        .context("Firetcp password must be given via env FIRETCP_PASSWORD")?;
                    tasks.push(
                        start_serving_tcp("firetcp", host, port, move |listener| async {
                            firetcp::server::run_server(listener, password.into()).await
                        })
                        .await?,
                    );
                }

                if let Some(port) = udpman_port {
                    tasks.push(
                        start_serving_udp("udpman", host, port, udpman::server::serve_socket)
                            .await?,
                    )
                }

                select_all(tasks).await.0
            }
            Command::Client {
                config,
                controller_host,
                controller_port,
            } => {
                let addr = SocketAddr::new(controller_host, controller_port);
                log::info!("Start controller at {addr}");
                run_controller(
                    bind_tcp(&Address::IP(addr))
                        .await
                        .context("Binding controller socket")?,
                    Path::new(&config),
                )
                .await
            }
        }
    })
}
