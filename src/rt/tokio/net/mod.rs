use std::net::SocketAddr;

pub use tokio::net::ToSocketAddrs as AsyncToSocketAddrs;

// pub type TcpListener = Compat<tokio::net::TcpListener>;
// pub type TcpStream = Compat<tokio::net::TcpStream>;

mod tcp;
mod udp;

pub use tcp::*;
pub use udp::*;

pub async fn resolve(a: impl AsyncToSocketAddrs) -> std::io::Result<Vec<SocketAddr>> {
    todo!()
}
