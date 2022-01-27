use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::geoip::CountryCode;
use crate::socks5::Address;

type Headers = Vec<u8>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ProxyRequestType {
    SocksTCP(Address),
    SocksUDP(Option<Address>),
    Http(Address, Headers),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum IPPolicyRule {
    Country { codes: Vec<CountryCode> },
    PrivateIP,
}

impl IPPolicyRule {
    pub fn matches(&self, ip: &IpAddr, cc: CountryCode) -> bool {
        match self {
            IPPolicyRule::Country { codes } => codes.contains(&cc),
            IPPolicyRule::PrivateIP => !ip.is_global(),
        }
    }

    pub fn matches_any<'a>(
        ip: &IpAddr,
        cc: CountryCode,
        mut rules: impl Iterator<Item = &'a IPPolicyRule>,
    ) -> bool {
        rules.find(|rule| rule.matches(ip, cc)).is_some()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IPPolicy {
    #[serde(default)]
    accept: Vec<IPPolicyRule>,
    #[serde(default)]
    reject: Vec<IPPolicyRule>,
    #[serde(default)]
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
        let result = match c {
            Some(c) => {
                (self.accept.is_empty() || IPPolicyRule::matches_any(ip, c, self.accept.iter()))
                    && (self.reject.is_empty()
                        || !IPPolicyRule::matches_any(ip, c, self.reject.iter()))
            }
            None => self.accept.is_empty(),
        };
        result
    }

    pub fn sort_by_preferences<T>(&self, c: &mut Vec<(T, Option<CountryCode>)>) {
        c.sort_unstable_by_key(|(_, c)| {
            c.and_then(|v| self.prefer.get(&v).cloned())
                .unwrap_or(usize::MAX)
        });
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProxyRequest {
    pub t: ProxyRequestType,
    pub policy: IPPolicy,
}

#[derive(Serialize, Deserialize, Debug)]
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_policy_works() {
        let us: CountryCode = "US".parse().unwrap();
        let nz: CountryCode = "NZ".parse().unwrap();

        assert!(IPPolicy::default().should_keep(&"8.8.8.8".parse().unwrap(), Some(us)));

        let us_only_policy = IPPolicy::new(
            vec![IPPolicyRule::Country { codes: vec![us] }],
            vec![],
            vec![],
        );

        assert!(us_only_policy.should_keep(&"8.8.8.8".parse().unwrap(), Some(us)));
        assert_eq!(
            us_only_policy.should_keep(&"65.9.139.97".parse().unwrap(), Some(nz)),
            false
        );

        let reject_nz_policy = IPPolicy::new(
            vec![],
            vec![IPPolicyRule::Country { codes: vec![nz] }],
            vec![],
        );
        assert!(reject_nz_policy.should_keep(&"8.8.8.8".parse().unwrap(), Some(us)));
        assert_eq!(
            reject_nz_policy.should_keep(&"65.9.139.97".parse().unwrap(), Some(nz)),
            false
        );
    }
}
