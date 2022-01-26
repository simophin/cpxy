use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use bincode::{Decode, Encode};

use crate::geoip::CountryCode;
use crate::socks5::Address;

type Headers = Vec<u8>;

#[derive(Encode, Decode, Debug, Clone)]
pub enum ProxyRequestType {
    SocksTCP(Address),
    SocksUDP(Option<Address>),
    Http(Address, Headers),
}

#[derive(Encode, Decode, Debug, Clone)]
pub enum IPPolicyRule {
    IsCountry(Vec<CountryCode>),
    IsPrivateIP,
}

impl IPPolicyRule {
    pub fn matches(&self, ip: &IpAddr, cc: CountryCode) -> bool {
        match self {
            IPPolicyRule::IsCountry(codes) => codes.contains(&cc),
            IPPolicyRule::IsPrivateIP => !ip.is_global(),
        }
    }

    pub fn matches_any<'a>(
        ip: &IpAddr,
        cc: CountryCode,
        mut rules: impl Iterator<Item = &'a IPPolicyRule>,
    ) -> bool {
        rules.find(|rule| rule.matches(ip, cc)).is_some()
    }

    pub fn matches_all<'a>(
        ip: &IpAddr,
        cc: CountryCode,
        mut rules: impl Iterator<Item = &'a IPPolicyRule>,
    ) -> bool {
        rules.find(|rule| !rule.matches(ip, cc)).is_none()
    }
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct IPPolicy {
    accept: Vec<IPPolicyRule>,
    reject: Vec<IPPolicyRule>,
    prefer: HashMap<CountryCode, usize>,
}

impl Default for IPPolicy {
    fn default() -> Self {
        Self {
            accept: Vec::new(),
            reject: Vec::new(),
            prefer: HashMap::new(),
        }
    }
}

impl IPPolicy {
    pub fn new(
        accept: Vec<IPPolicyRule>,
        reject: Vec<IPPolicyRule>,
        prefer: Vec<CountryCode>,
    ) -> Self {
        Self {
            accept,
            reject,
            prefer: prefer
                .into_iter()
                .enumerate()
                .map(|(index, c)| (c, index))
                .collect(),
        }
    }

    pub fn should_keep(&self, ip: &IpAddr, c: Option<CountryCode>) -> bool {
        match c {
            Some(c) => {
                (self.accept.is_empty() || IPPolicyRule::matches_any(ip, c, self.accept.iter()))
                    && (self.reject.is_empty()
                        || !IPPolicyRule::matches_any(ip, c, self.reject.iter()))
            }
            None => self.accept.is_empty(),
        }
    }

    pub fn sort_by_preferences<T>(&self, c: &mut Vec<(T, Option<CountryCode>)>) {
        c.sort_by_key(|(_, c)| {
            c.and_then(|v| self.prefer.get(&v).cloned())
                .unwrap_or(usize::MAX)
        });
    }
}

#[derive(Encode, Decode, Debug)]
pub struct ProxyRequest {
    pub t: ProxyRequestType,
    pub policy: IPPolicy,
}

#[derive(Encode, Decode, Debug)]
pub enum ProxyResult {
    Granted {
        bound_address: SocketAddr,
    },
    ErrHostRejected {
        resolved: Vec<(IpAddr, Option<CountryCode>)>,
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
