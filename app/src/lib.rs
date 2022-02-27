extern crate core;

mod broadcast;
mod buf;
mod cipher;
mod client;
mod counter;
mod dns;
mod fetch;
mod handshake;
mod http;
mod parse;
mod pattern;
mod proxy;
pub mod server;
mod shared_udp;
mod socks4;
pub mod socks5;
mod stream;
mod tls;
#[cfg(target_os = "linux")]
pub mod transparent;
mod url;
mod utils;

#[cfg(target_os = "android")]
mod jni_export;

pub mod io;
#[cfg(test)]
mod test;

mod abp;
pub mod config;
pub mod controller;
mod geoip;
mod udp_relay;
