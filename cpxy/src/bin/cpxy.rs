use anyhow::Context;
use async_shutdown::Shutdown;
use clap::{Parser, Subcommand};
use std::future::Future;
// use cpxy::controller::run_controller;
// use cpxy::controller_config::fs::FileConfigProvider;
use cpxy::io::bind_tcp;
use cpxy::protocol::tcpman;
use cpxy::socks5::Address;
// use futures::Future;
use cpxy::protocol::tcpman::Tcpman;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use tokio::net::TcpListener;
use tokio::signal::ctrl_c;
use tokio::spawn;

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
        #[clap(long)]
        firetcp_port: Option<u16>,
    },

    #[clap()]
    Client {
        #[clap(long)]
        /// Path to the configuration file. Can be a URL or a local file.
        config: String,

        #[clap(default_value = "127.0.0.1", long)]
        controller_host: IpAddr,

        #[clap(default_value_t = 4000, long)]
        controller_port: u16,
    },
}

async fn start_serving_tcp<Fut: Future<Output = anyhow::Result<()>> + Send + Sync + 'static>(
    shutdown: Shutdown,
    service_name: String,
    host: IpAddr,
    port: u16,
    run: impl FnOnce(Shutdown, TcpListener) -> Fut + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let addr = SocketAddr::new(host, port);
    let listener = bind_tcp(&Address::IP(addr))
        .await
        .with_context(|| format!("Binding on {addr} for {service_name}"))?;
    log::info!("{service_name} started on {addr}");
    spawn(async move {
        if let Some(Err(e)) = shutdown.wrap_cancel(run(shutdown.clone(), listener)).await {
            log::error!("{service_name} error: {e}");
        }
    });

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }

    env_logger::init();

    let Cli { cmd } = Cli::parse();

    let shutdown = Shutdown::new();

    match cmd {
        Command::Server {
            host,
            tcpman_port,
            firetcp_port,
        } => {
            if let Some(port) = tcpman_port {
                let tcpman_password = std::env::var("TCPMAN_PASSWORD")
                    .context("Tcpman password must be given via env TCPMAN_PASSWORD")?;

                let tcpman_key =
                    Tcpman::derive_key(&tcpman_password).context("deriving key from password")?;

                start_serving_tcp(
                    shutdown.clone(),
                    "tcpman".to_string(),
                    host,
                    port,
                    move |shutdown, listener| {
                        tcpman::server::run_tcpman_server(shutdown, tcpman_key, listener)
                    },
                )
                .await?;
            }

            // if let Some(port) = firetcp_port {
            //     let password = std::env::var("FIRETCP_PASSWORD")
            //         .context("Firetcp password must be given via env FIRETCP_PASSWORD")?;
            //     start_serving_tcp(
            //         shutdown.clone(),
            //         "firetcp".to_string(),
            //         host,
            //         port,
            //         move |shutdown, listener| async {
            //             firetcp::server::run_server(shutdown, listener, password.into()).await
            //         },
            //     )
            //     .await?;
            // }

            let _ = ctrl_c().await;
            shutdown.shutdown();
            shutdown.wait_shutdown_complete().await;
            Ok(())
        }

        Command::Client {
            config,
            controller_host,
            controller_port,
        } => {
            let addr = SocketAddr::new(controller_host, controller_port);
            log::info!("Start controller at {addr}");

            todo!()

            // let (provider, receiver) =
            //     FileConfigProvider::new(Path::new(&config).to_path_buf()).await?;
            //
            // run_controller(
            //     bind_tcp(&Address::IP(addr))
            //         .await
            //         .context("Binding controller socket")?,
            //     receiver,
            //     Box::new(provider),
            // )
            // .await
        }
    }
}
