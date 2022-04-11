use std::{
    borrow::Cow,
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};

use serde::{Deserialize, Serialize};

use crate::{http::HttpRequest, socks5::Address};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ProxyRequest<'a> {
    TCP {
        dst: Address<'a>,
    },
    UDP {
        initial_dst: Address<'a>,
        initial_data: Cow<'a, [u8]>,
    },
    DNS {
        domains: Vec<String>,
    },
    HTTP {
        dst: Address<'a>,
        https: bool,
        req: HttpRequest<'a>,
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
