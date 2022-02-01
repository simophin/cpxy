use crate::abp::matches_gfw_list;
use crate::geoip::{find_geoip, CountryCode};
use crate::socks5::Address;
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::ops::Deref;
use std::sync::{Arc, RwLock};
use std::time::Instant;

pub type LastVisitMap = Arc<RwLock<HashMap<String, Instant>>>;

#[derive(Debug, Serialize, Deserialize)]
pub struct UpstreamConfig {
    pub address: Address,
    #[serde(default)]
    pub accept: Vec<CountryCode>,
    #[serde(default)]
    pub reject: Vec<CountryCode>,
    #[serde(default)]
    pub priority: u16,
    #[serde(default)]
    pub match_gfw: bool,
    #[serde(default)]
    pub match_networks: Vec<IpNetwork>,
}

impl UpstreamConfig {
    fn matches_network(&self, ip: &IpAddr) -> bool {
        self.match_networks
            .iter()
            .find(|n| n.contains(ip.clone()))
            .is_some()
    }

    fn matches(&self, target: &Address) -> bool {
        match target {
            Address::IP(addr) => {
                // Match specified network first
                if self.matches_network(&addr.ip()) {
                    return true;
                }

                let country_code = match find_geoip(&addr.ip()) {
                    Some(c) => c,
                    _ => return false,
                };

                (self.accept.is_empty() || self.accept.contains(&country_code))
                    && (self.reject.is_empty() || !self.reject.contains(&country_code))
            }

            v if self.match_gfw => matches_gfw_list(v),
            _ => false,
        }
    }
}

fn default_socks5_udp_host() -> String {
    "0.0.0.0".to_string()
}

fn default_socks5_address() -> Address {
    "127.0.0.1:5000".parse().unwrap()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientConfig {
    pub upstreams: HashMap<String, UpstreamConfig>,

    #[serde(default = "default_socks5_address")]
    pub socks5_address: Address,

    #[serde(default = "default_socks5_udp_host")]
    pub socks5_udp_host: String,
}

impl ClientConfig {
    fn calc_last_visit_score(
        last_visit: &impl Deref<Target = HashMap<String, Instant>>,
        upstream_name: &String,
    ) -> usize {
        (match last_visit.get(upstream_name) {
            Some(i) => i.elapsed().as_millis().try_into().unwrap_or(u16::MAX),
            _ => u16::MAX,
        }) as usize
    }

    pub fn find_best_upstream(
        &self,
        last_visit: LastVisitMap,
        target: &Address,
    ) -> Option<(&str, &UpstreamConfig)> {
        let mut upstreams: Vec<(&str, &UpstreamConfig, usize)> = {
            let last_visit = last_visit.read().ok()?;
            // Find suitable upstreams first
            self.upstreams
                .iter()
                .filter(|(_, c)| c.matches(target))
                .map(|(n, c)| {
                    (
                        n.as_str(),
                        c,
                        (u16::MAX - c.priority) as usize
                            + Self::calc_last_visit_score(&last_visit, n),
                    )
                })
                .collect()
        };

        upstreams.sort_by_key(|(_, _, score)| *score);
        let result = upstreams.last().map(|(n, c, _)| (*n, *c));

        if let Some((name, _)) = result.as_ref() {
            if let Ok(mut v) = last_visit.try_write() {
                match v.get_mut(*name) {
                    Some(v) => *v = Instant::now(),
                    None => {
                        v.insert(name.to_string(), Instant::now());
                    }
                };
            }
        }

        result
    }
}
