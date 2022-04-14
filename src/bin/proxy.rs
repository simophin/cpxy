use anyhow::Context;
use clap::{Parser, Subcommand};
use proxy::controller::run_controller;
use proxy::io::bind_tcp;
use proxy::rt;
use proxy::server::run_server;
use proxy::socks5::Address;
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
    #[clap()]
    Server {
        /// The address to listen on
        #[clap(default_value = "0.0.0.0", long)]
        host: IpAddr,
        /// The HTTP port to listen on
        #[clap(default_value_t = 80, long)]
        port: u16,
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

fn main() -> anyhow::Result<()> {
    rt::block_on(async move {
        if std::env::var_os("RUST_LOG").is_none() {
            std::env::set_var("RUST_LOG", "info");
        }

        env_logger::init();

        let Cli { cmd } = Cli::parse();
        match cmd {
            Command::Server { host, port } => {
                let addr = SocketAddr::new(host, port);
                log::info!("Start server at {addr}");
                run_server(
                    bind_tcp(&Address::IP(addr))
                        .await
                        .context("Binding server socket")?,
                )
                .await
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
