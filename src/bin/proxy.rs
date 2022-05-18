use anyhow::Context;
use clap::{Parser, Subcommand};
use proxy::controller::run_controller;
use proxy::io::bind_tcp;
use proxy::protocol::{tcpman, udpman};
use proxy::rt;
use proxy::rt::net::UdpSocket;
use proxy::socks5::Address;
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
        #[clap(default_value_t = 80, long)]
        tcp_port: u16,
        /// The UDPMan port to listen on
        #[clap(default_value_t = 3000, long)]
        udp_port: u16,
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
    #[clap()]
    Bench {
        #[clap(long)]
        /// Path to the configuration file
        config: String,
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
            Command::Server {
                host,
                tcp_port,
                udp_port,
            } => {
                let tcp_addr = SocketAddr::new(host, tcp_port);
                let udp_addr = SocketAddr::new(host, udp_port);
                log::info!("Start server at TCPMan://{tcp_addr}, UDPMan://{udp_addr}");

                let tcp_man_server_socket = bind_tcp(&Address::IP(tcp_addr))
                    .await
                    .context("Binding TCPMan server socket")?;

                let udp_man_server_socket = UdpSocket::bind(udp_addr)
                    .await
                    .context("Binding UDPMan server socket")?;

                proxy::utils::race(
                    tcpman::server::run_server(tcp_man_server_socket),
                    udpman::server::serve_socket(udp_man_server_socket),
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
            Command::Bench { config: _config } => {
                todo!()
                // proxy::bench_client::run_perf_tests(Path::new(&config)).await
            }
        }
    })
}
