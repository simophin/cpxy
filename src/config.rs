use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::UNIX_EPOCH;

use crate::client::ClientStatistics;
use crate::dns::DnsCache;
use crate::geoip::find_geoip;
use crate::protocol::{DynamicProtocol, ProxyRequest, Stats};
use crate::rule::{PacketDestination, RuleExecutionResult, RuleString};
use crate::socks5::Address;

pub const fn default_upstream_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpstreamConfig {
    pub protocol: DynamicProtocol,
    pub groups: Option<HashSet<String>>,
    #[serde(default = "default_upstream_enabled")]
    pub enabled: bool,
}

const fn default_socks5_udp_host() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

fn default_socks5_address() -> SocketAddr {
    "127.0.0.1:5000".parse().unwrap()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClientConfig {
    #[serde(default)]
    pub upstreams: HashMap<String, UpstreamConfig>,

    #[serde(default = "default_socks5_address")]
    pub socks5_address: SocketAddr,

    #[serde(default = "default_socks5_udp_host")]
    pub socks5_udp_host: IpAddr,

    #[serde(default)]
    pub fwmark: Option<u32>,

    #[serde(default)]
    pub traffic_rules: RuleString,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            socks5_address: default_socks5_address(),
            socks5_udp_host: default_socks5_udp_host(),
            upstreams: Default::default(),
            fwmark: None,
            traffic_rules: Default::default(),
        }
    }
}

impl ClientConfig {
    fn calc_last_visit_score(stats: &ClientStatistics, upstream_name: &String) -> usize {
        stats
            .get_upstream(upstream_name, |stat| {
                let last = stat.last_activity.get() as u64;
                let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
                now.checked_sub(last)
                    .unwrap_or(0)
                    .try_into()
                    .unwrap_or(u16::MAX)
            })
            .unwrap_or(u16::MAX) as usize
    }

    // Sorted by score MIN -> MAX
    pub fn find_best_upstream(
        &self,
        stats: &ClientStatistics,
        req: &ProxyRequest,
    ) -> anyhow::Result<Vec<(&str, &DynamicProtocol)>> {
        let pkt_dst = match &req.dst {
            Address::IP(addr) => PacketDestination::IP {
                addr: addr.clone(),
                country_code: find_geoip(&addr.ip()),
                resolved_host: DnsCache::global().get(&addr.ip()).into_iter().collect(),
            },

            Address::Name { host, port } => PacketDestination::Domain {
                hostname: host.as_ref(),
                port: *port,
                resolved_ips: Default::default(),
            },
        };

        let action = self
            .traffic_rules
            .execute_rules(&pkt_dst, req.initial_data.as_ref().map(|b| b.as_ref()))?;

        let mut upstreams: Vec<(&str, &UpstreamConfig, usize)> = match action {
            None => self
                .upstreams
                .iter()
                .filter_map(|(n, c)| {
                    if !c.enabled {
                        return None;
                    }

                    Some((n.as_str(), c, Self::calc_last_visit_score(stats, n)))
                })
                .collect(),
            Some(RuleExecutionResult::Proxy(name)) => self
                .upstreams
                .get(name)
                .into_iter()
                .filter(|c| c.enabled)
                .map(move |config| (name, config, 0))
                .collect(),
            Some(RuleExecutionResult::ProxyGroup(name)) => self
                .upstreams
                .iter()
                .filter_map(|(n, c)| {
                    if !c.enabled {
                        return None;
                    }

                    match &c.groups {
                        Some(groups) if !groups.contains(name) => {
                            return None;
                        }
                        _ => {}
                    }

                    Some((n.as_str(), c, Self::calc_last_visit_score(stats, n)))
                })
                .collect(),
            Some(RuleExecutionResult::Reject) => Default::default(),
        };

        upstreams.sort_by_key(|(_, _, score)| *score);
        let result = upstreams.into_iter().map(|(n, c, _)| (n, c)).collect();
        Ok(result)
    }
}
