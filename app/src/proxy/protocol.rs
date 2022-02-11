use bytes::Bytes;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};

use serde::{Deserialize, Serialize};

use crate::socks5::Address;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ProxyRequest {
    TCP { dst: Address },
    UDP,
    DNS { domains: Vec<String> },
    Http { dst: Address, request: Bytes },
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ProxyResult {
    Granted {
        bound_address: SocketAddr,
    },
    DNSResolved {
        addresses: HashMap<String, Vec<IpAddr>>,
    },
    ErrHostNotFound,
    ErrTimeout,
    ErrGeneric {
        msg: String,
    },
}

impl std::fmt::Display for ProxyResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for ProxyResult {}
