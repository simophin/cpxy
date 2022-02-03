use clap::{AppSettings, Parser, Subcommand};
use proxy::controller::run_controller;
use proxy::server::run_server;
use smol::net::TcpListener;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

/// SOCKS5 over HTTPs
#[derive(Parser)]
struct Cli {
    #[clap(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    #[clap(setting(AppSettings::ArgRequiredElseHelp))]
    Server {
        /// The address to listen on
        #[clap(default_value = "0.0.0.0", long)]
        host: String,
        /// The HTTP port to listen on
        #[clap(default_value_t = 80, long)]
        port: u16,
    },

    #[clap(setting(AppSettings::ArgRequiredElseHelp))]
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

fn main() -> anyhow::Result<()> {
    smol::block_on(async move {
        if std::env::var_os("RUST_LOG").is_none() {
            std::env::set_var("RUST_LOG", "info");
        }

        env_logger::init();

        let Cli { cmd } = Cli::parse();
        match cmd {
            Command::Server { host, port } => {
                let listen_address = format!("{host}:{port}");
                log::info!("Start server at {listen_address}");
                run_server(TcpListener::bind(listen_address).await?).await
            }
            Command::Client {
                config,
                controller_host,
                controller_port,
            } => {
                run_controller(
                    SocketAddr::new(controller_host, controller_port),
                    Path::new(&config),
                )
                .await
            }
        }
    })
}
