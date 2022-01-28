#![feature(ip)]

mod cipher;
pub mod client;
mod handshake;
mod parse;
mod proxy;
pub mod server;
mod socks5;
mod utils;

#[cfg(target_os = "android")]
mod jni_export;

mod domains;
pub mod geoip;
mod io;
#[cfg(test)]
mod test;

pub use crate::proxy::protocol::{IPPolicy, IPPolicyRule};
