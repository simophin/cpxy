use clap::{AppSettings, Parser, Subcommand};
use futures_lite::future::race;
use proxy::client::run_client;
use proxy::config::{ClientConfig, UpstreamConfig};
use proxy::server::run_server;
use smol::net::TcpListener;
use std::collections::HashMap;
use std::fs::File;
use std::sync::Arc;

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
    },

    #[clap(setting(AppSettings::ArgRequiredElseHelp))]
    Standalone {
        /// The SOCKS5 host to listen on
        #[clap(default_value = "127.0.0.1", long)]
        socks5_host: String,

        /// The SOCKS5 UDP relay host to listen on
        #[clap(default_value = "127.0.0.1", long)]
        socks5_udp_host: String,

        /// The SOCKS5 port to listen on
        #[clap(default_value_t = 5000, long)]
        socks5_port: u16,
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
            Command::Client { config } => {
                let config: ClientConfig =
                    serde_yaml::from_reader(File::open(config).expect("To open {config}"))
                        .expect("Valid config file");
                let listen_address = config.socks5_address.to_string();
                log::info!("Start client at {listen_address}");

                run_client(TcpListener::bind(listen_address).await?, Arc::new(config)).await
            }

            Command::Standalone {
                socks5_host,
                socks5_port,
                socks5_udp_host,
            } => {
                let server = TcpListener::bind("localhost:0").await?;
                let listen_address = server.local_addr()?;
                let socks_listen_address = format!("{socks5_host}:{socks5_port}");
                let socks_server = TcpListener::bind(&socks_listen_address).await?;
                log::info!("Started socks5 at {socks_listen_address}");

                let mut upstreams = HashMap::new();
                upstreams.insert(
                    "only".to_string(),
                    UpstreamConfig {
                        address: listen_address.into(),
                        match_networks: Default::default(),
                        accept: Default::default(),
                        reject: Default::default(),
                        priority: 0,
                        match_gfw: false,
                    },
                );

                race(
                    run_server(server),
                    run_client(
                        socks_server,
                        Arc::new(ClientConfig {
                            socks5_udp_host,
                            socks5_address: socks_listen_address.parse().unwrap(),
                            upstreams,
                        }),
                    ),
                )
                .await
            }
        }
    })
}
