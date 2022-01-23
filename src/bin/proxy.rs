use clap::{AppSettings, Parser, Subcommand};
use proxy::client::run_client;
use proxy::server::run_server;
use smol::net::TcpListener;

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

        /// The remote server's host
        #[clap(long)]
        remote_host: String,

        /// The remote server's port
        #[clap(default_value_t = 80, long)]
        remote_port: u16,
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
            Command::Server { host, port } => run_server(&format!("{host}:{port}")).await,
            Command::Client {
                socks5_host,
                socks5_port,
                remote_host,
                remote_port,
            } => {
                let listen_address = format!("{socks5_host}:{socks5_port}");
                log::info!("Start client at {listen_address}");

                run_client(
                    TcpListener::bind(listen_address).await?,
                    remote_host.as_str(),
                    remote_port,
                )
                .await
            }
        }
    })
}
