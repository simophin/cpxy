#![feature(future_join, future_poll_fn)]

extern crate core;

mod cipher;
mod client;
mod handshake;
mod parse;
mod proxy;
pub mod server;
mod socks5;
mod utils;

// #[cfg(target_os = "android")]
// mod jni_export;

mod io;
#[cfg(test)]
mod test;

mod abp;
pub mod config;
pub mod controller;
mod geoip;
mod udp_relay;
