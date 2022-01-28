use bytes::Bytes;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::socks5::Address;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ProxyRequest {
    TCP { dst: Address },
    UDP,
    Http { dst: Address, request: Bytes },
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ProxyResult {
    Granted { bound_address: SocketAddr },
    ErrHostNotFound,
    ErrTimeout,
    ErrGeneric { msg: String },
}

impl std::fmt::Display for ProxyResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for ProxyResult {}
