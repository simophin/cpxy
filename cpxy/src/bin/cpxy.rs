use anyhow::Context;
use async_shutdown::Shutdown;
use clap::{Parser, Subcommand};
use cpxy::protocol;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use tokio::net::TcpListener;
use tokio::signal::ctrl_c;
use tokio::spawn;

#[derive(Parser)]
struct Cli {
    #[clap(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve tcpman server. TCPMAN_PASSWORD must be given as an environment variable.
    #[clap()]
    ServeTcpman {
        /// The address to listen on
        #[clap(default_value = "0.0.0.0", long)]
        host: IpAddr,

        #[clap(default_value = "8009", long)]
        port: NonZeroU16,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }

    env_logger::init();

    let Cli { cmd } = Cli::parse();

    let shutdown = Shutdown::new();

    match cmd {
        Command::ServeTcpman { host, port } => {
            let tcpman_password = std::env::var("TCPMAN_PASSWORD")
                .context("Tcpman password must be given via env TCPMAN_PASSWORD")?;

            let listener = TcpListener::bind((host, port.get()))
                .await
                .with_context(|| format!("Binding tcp on {host}:{port}"))?;

            log::info!("tcpman started on {host}:{port}");

            spawn(protocol::server::run_server(
                shutdown.clone(),
                "tcpman",
                listener,
                protocol::tcpman::server::TcpmanAcceptor(
                    tcpman_password.parse().context("Parsing tcpman password")?,
                ),
                protocol::direct::Direct::default(),
            ));

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
