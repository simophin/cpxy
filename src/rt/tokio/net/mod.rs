pub use tokio::net::ToSocketAddrs as AsyncToSocketAddrs;

mod tcp;
mod udp;

pub use tcp::*;
pub use udp::*;

pub use tokio::net::lookup_host as resolve;
