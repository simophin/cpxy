use std::net::{IpAddr, SocketAddr};

use anyhow::Context;
use clap::{Parser, Subcommand};
use proxy::{
    io::TcpListener,
    rsocks::{client::run_client, server::run_server},
    rt::block_on,
    socks5::Address,
};

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

        /// The SOCKS address to listen on
        #[clap(default_value = "127.0.0.1", long)]
        socks_host: IpAddr,

        /// The SOCKS port to listen on
        #[clap(default_value = "1080", long)]
        socks_port: u16,
    },

    #[clap()]
    Client {
        /// The URL of server, e.g. http://xxx or https://xxx
        #[clap()]
        url: String,
    },
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    block_on(async move {
        let Cli { cmd } = Cli::parse();
        match cmd {
            Command::Server {
                host,
                port,
                socks_host,
                socks_port,
            } => {
                let control_listener = TcpListener::bind(&Address::IP(SocketAddr::new(host, port)))
                    .await
                    .context("Bind server socket")?;
                log::info!("Control socket listening on {host}:{port}");
                let socks_listener =
                    TcpListener::bind(&Address::IP(SocketAddr::new(socks_host, socks_port)))
                        .await
                        .context("Bind SOCK5 socket")?;
                log::info!("Socks5 socket listening on {socks_host}:{socks_port}");
                run_server(control_listener, socks_listener).await
            }

            Command::Client { url } => run_client(url).await,
        }
    })
}
