use anyhow::Context;
use async_shutdown::Shutdown;
use clap::{Parser, Subcommand};
use cpxy::addr::Address;
use cpxy::http::parse_response;
use cpxy::http::writer::RequestWriter;
use cpxy::protocol;
use cpxy::protocol::http::auth::{BasicAuthProvider, BasicAuthSettings};
use cpxy::protocol::tcpman::Tcpman;
use cpxy::protocol::{Protocol, ProtocolAcceptor};
use cpxy::tls::TlsStream;
use hyper::{header, Method};
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::BufReader;
use tokio::net::TcpListener;
use tokio::signal::ctrl_c;
use tokio::spawn;
use tokio::time::timeout;

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

    /// Serve http server. To set up basic auth, set environment HTTP_PROXY_USER and HTTP_PROXY_PASSWORD
    #[clap()]
    ServeHttpProxy {
        /// The address to listen on
        #[clap(default_value = "0.0.0.0", long)]
        host: IpAddr,

        #[clap(default_value = "8080", long)]
        port: NonZeroU16,
    },

    /// Serve socks5 server.
    #[clap()]
    ServeSocks5 {
        /// The address to listen on
        #[clap(default_value = "0.0.0.0", long)]
        host: IpAddr,

        #[clap(default_value = "3128", long)]
        port: NonZeroU16,
    },

    #[clap()]
    TestTcpman {
        #[clap(long)]
        password: String,

        #[clap(default_value_t = false, long)]
        tls: bool,

        #[clap()]
        addr: Address,
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

    match cmd {
        Command::ServeTcpman { host, port } => {
            let acceptor = protocol::tcpman::server::TcpmanAcceptor(
                std::env::var("TCPMAN_PASSWORD")
                    .expect("to have set env TCPMAN_PASSWORD")
                    .parse()
                    .expect("valid password"),
            );

            serve_protocol_server("tcpman", host, port, acceptor).await;
            Ok(())
        }

        Command::ServeHttpProxy { host, port } => {
            let auth = match (
                std::env::var("HTTP_PROXY_USER"),
                std::env::var("HTTP_PROXY_PASSWORD"),
            ) {
                (Ok(user), Ok(password)) if !user.is_empty() && !password.is_empty() => {
                    Some(BasicAuthSettings { user, password })
                }
                (Err(_), Err(_)) => None,
                _ => panic!(
                    "Must specify both HTTP_PROXY_USER and HTTP_PROXY_PASSWORD or leave them unset"
                ),
            };

            let acceptor = protocol::http::server::HttpProxyAcceptor {
                auth_provider: auth.map(|settings| Arc::new(BasicAuthProvider(settings))),
            };

            serve_protocol_server("http_proxy", host, port, acceptor).await;
            Ok(())
        }

        Command::ServeSocks5 { host, port } => {
            let acceptor = protocol::socks5::server::Socks5Acceptor::default();
            serve_protocol_server("socks5_proxy", host, port, acceptor).await;
            Ok(())
        }

        Command::Client {
            config: _,
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

        Command::TestTcpman {
            password,
            tls,
            addr,
        } => {
            timeout(Duration::from_secs(10), test_tcpman(addr, tls, password))
                .await
                .expect("To not timeout");

            Ok(())
        }
    }
}

async fn serve_protocol_server(
    name: &'static str,
    host: IpAddr,
    port: NonZeroU16,
    acceptor: impl ProtocolAcceptor + Clone + Send + Sync + 'static,
) {
    let listener = TcpListener::bind((host, port.get()))
        .await
        .with_context(|| format!("Binding tcp on {host}:{port}"))
        .unwrap();

    let shutdown = Shutdown::new();

    log::info!("{name} started on {host}:{port}");

    let task = spawn(protocol::server::run_server(
        shutdown.clone(),
        name,
        listener,
        acceptor,
        protocol::direct::Direct::default(),
    ));

    let _ = ctrl_c().await;
    log::info!("{name} shutting down...");
    shutdown.shutdown();
    shutdown.wait_shutdown_complete().await;

    timeout(Duration::from_secs(10), task)
        .await
        .expect("timeout waiting for shutting down")
        .expect("Server to complete")
        .expect("server to exit successfully");
}

async fn test_tcpman(addr: Address, tls: bool, password: String) {
    let p = Tcpman::new(addr, tls, password).expect("Creating client");
    let upstream = p
        .new_stream(
            &"www.baidu.com:443".parse().expect("parsing address"),
            &Default::default(),
            None,
        )
        .await
        .expect("connecting to upstream");

    let mut upstream = BufReader::new(
        TlsStream::connect_tls("www.baidu.com", upstream)
            .await
            .expect("Connect to TLS"),
    );

    let mut writer = RequestWriter::write(Method::GET, "/");
    writer.write_header(header::HOST, "www.baidu.com");
    writer
        .to_async(&mut upstream)
        .await
        .expect("To write request");

    let code = parse_response(&mut upstream, |r| r.code.context("status code"))
        .await
        .expect("to parse response");

    assert_eq!(code, 200);
}
