use either::Either;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use crate::domains::matches_gfw;
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(tag = "type")]
pub enum RouteDestination {
    Country(CountryCode),
    GFWList,
    PrivateIP,
}

impl RouteDestination {
    pub fn matches(&self, test: (&Address, Option<CountryCode>)) -> bool {
        match (self, test) {
            (RouteDestination::Country(c), (_, Some(test_c))) => c == &test_c,
            (RouteDestination::PrivateIP, (Address::IP(addr), _)) => !addr.ip().is_global(),
            (RouteDestination::GFWList, (addr, _)) if matches!(addr, Address::Name { .. }) => {
                matches_gfw(&addr)
            }
            _ => false,
        }
    }

    pub fn matches_any<'a>(
        test: (&Address, Option<CountryCode>),
        mut rules: impl Iterator<Item = &'a RouteDestination>,
    ) -> bool {
        let (addr, cc) = test;
        rules.find(|rule| rule.matches((addr, cc))).is_some()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IPPolicy {
    #[serde(default)]
    accept: Vec<RouteDestination>,
    #[serde(default)]
    reject: Vec<RouteDestination>,
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
        accept: Vec<RouteDestination>,
        reject: Vec<RouteDestination>,
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

    pub fn should_keep(&self, ip: &Address, c: Option<CountryCode>) -> bool {
        (self.accept.is_empty() || RouteDestination::matches_any((ip, c), self.accept.iter()))
            && (self.reject.is_empty()
                || !RouteDestination::matches_any((ip, c), self.reject.iter()))
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

        assert!(IPPolicy::default().should_keep(&"8.8.8.8:80".parse().unwrap(), Some(us)));

        let us_only_policy = IPPolicy::new(vec![RouteDestination::Country(us)], vec![], vec![]);

        assert!(us_only_policy.should_keep(&"8.8.8.8:53".parse().unwrap(), Some(us)));
        assert_eq!(
            us_only_policy.should_keep(&"65.9.139.97:80".parse().unwrap(), Some(nz)),
            false
        );

        let reject_nz_policy = IPPolicy::new(vec![], vec![RouteDestination::Country(nz)], vec![]);
        assert!(reject_nz_policy.should_keep(&"8.8.8.8:443".parse().unwrap(), Some(us)));
        assert_eq!(
            reject_nz_policy.should_keep(&"65.9.139.97:80".parse().unwrap(), Some(nz)),
            false
        );
    }
}
