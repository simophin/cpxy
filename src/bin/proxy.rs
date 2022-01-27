use clap::{AppSettings, Parser, Subcommand};
use futures_lite::future::race;
use proxy::client::{run_client, ClientConfig};
use proxy::geoip::CountryCode;
use proxy::server::run_server;
use proxy::{IPPolicy, IPPolicyRule};
use smol::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

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
        /// The SOCKS5 host to listen on
        #[clap(default_value = "127.0.0.1", long)]
        socks5_host: String,

        /// The SOCKS5 port to listen on
        #[clap(default_value_t = 5000, long)]
        socks5_port: u16,

        /// The SOCKS5 UDP relay host to listen on
        #[clap(default_value = "127.0.0.1", long)]
        socks5_udp_host: String,

        /// The remote server's host
        #[clap(long)]
        remote_host: String,

        /// The remote server's port
        #[clap(default_value_t = 80, long)]
        remote_port: u16,

        /// The countries to proxy. If specified only these countries will be proxied.
        #[clap(long, multiple_occurrences(true))]
        accept_country: Vec<CountryCode>,

        /// The countries not to proxy.
        #[clap(long, multiple_occurrences(true))]
        reject_country: Vec<CountryCode>,

        /// The order of countries preferred to proxy through
        #[clap(long, multiple_occurrences(true))]
        prefer_country: Vec<CountryCode>,
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

        /// The countries to proxy. If specified only these countries will be proxied.
        #[clap(long, multiple_occurrences(true))]
        accept_country: Vec<CountryCode>,

        /// The countries not to proxy.
        #[clap(long, multiple_occurrences(true))]
        reject_country: Vec<CountryCode>,

        /// The order of countries preferred to proxy through
        #[clap(long, multiple_occurrences(true))]
        prefer_country: Vec<CountryCode>,
    },
}

fn create_remote_policy(
    accept: Vec<CountryCode>,
    reject: Vec<CountryCode>,
    prefer: Vec<CountryCode>,
) -> IPPolicy {
    IPPolicy::new(
        if accept.is_empty() {
            vec![]
        } else {
            vec![IPPolicyRule::Country { codes: accept }]
        },
        if reject.is_empty() {
            vec![]
        } else {
            vec![IPPolicyRule::Country { codes: reject }]
        },
        prefer,
    )
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
                socks5_host,
                socks5_port,
                socks5_udp_host,
                remote_host,
                remote_port,
                accept_country: accept_countries,
                reject_country: reject_countries,
                prefer_country: prefer_countries,
            } => {
                let listen_address = format!("{socks5_host}:{socks5_port}");
                log::info!("Start client at {listen_address}");

                run_client(
                    TcpListener::bind(listen_address).await?,
                    Arc::new(ClientConfig {
                        local_policy: Default::default(),
                        socks5_udp_host,
                        upstream_policy: create_remote_policy(
                            accept_countries,
                            reject_countries,
                            prefer_countries,
                        ),
                        upstream_timeout: Duration::from_secs(3),
                        upstream: format!("{remote_host}:{remote_port}")
                            .parse()
                            .expect("To parse remote address:port"),
                    }),
                )
                .await
            }

            Command::Standalone {
                socks5_host,
                socks5_port,
                socks5_udp_host,
                accept_country: accept_countries,
                reject_country: reject_countries,
                prefer_country: prefer_countries,
            } => {
                let server = TcpListener::bind("localhost:0").await?;
                let listen_address = server.local_addr()?;
                let socks_listen_address = format!("{socks5_host}:{socks5_port}");
                let socks_server = TcpListener::bind(&socks_listen_address).await?;
                log::info!("Started socks5 at {socks_listen_address}");

                race(
                    run_server(server),
                    run_client(
                        socks_server,
                        Arc::new(ClientConfig {
                            local_policy: Default::default(),
                            socks5_udp_host,
                            upstream_policy: create_remote_policy(
                                accept_countries,
                                reject_countries,
                                prefer_countries,
                            ),
                            upstream_timeout: Duration::from_secs(3),
                            upstream: listen_address.into(),
                        }),
                    ),
                )
                .await
            }
        }
    })
}
