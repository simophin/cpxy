mod chunked;
mod client;
mod http;
mod parse;
mod server;
mod socks5;
// pub mod udp;

use futures::{select, FutureExt};

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let upstream_host = "l.fanchao.dev";
    let upstream_port = 443;

    select! {
        c = client::run_client("127.0.0.1:8081", upstream_host, upstream_port).fuse() => c,
        s = server::run_server("0.0.0.0:4200").fuse() => s,
    }
}
