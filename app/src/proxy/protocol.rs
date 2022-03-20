use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};

use serde::{Deserialize, Serialize};

use crate::{http::HttpRequest, socks5::Address};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ProxyRequest {
    TCP {
        dst: Address<'static>,
    },
    UDPConn {
        dst: SocketAddr,
    },
    UDP,
    DNS {
        domains: Vec<String>,
    },
    HTTP {
        dst: Address<'static>,
        https: bool,
        req: HttpRequest<'static>,
    },
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ProxyResult {
    Granted {
        bound_address: Option<SocketAddr>,
        solved_addresses: Option<HashMap<String, Vec<IpAddr>>>,
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
