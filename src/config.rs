use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::UNIX_EPOCH;

use ipnetwork::IpNetwork;
use serde_with::{DeserializeFromStr, SerializeDisplay};

use crate::abp::{adblock_list_engine, gfw_list_engine};
use crate::client::ClientStatistics;
use crate::geoip::{find_geoip, CountryCode};
use crate::pattern::Pattern;
use crate::protocol::{
    direct, socks5, tcpman, udpman, AsyncStream, BoxedSink, BoxedStream, Protocol, Stats,
    TrafficType,
};
use crate::socks5::Address;

#[derive(Debug, Clone, DeserializeFromStr, SerializeDisplay)]
pub enum TrafficMatchRule {
    GeoIP(CountryCode),
    Network(IpNetwork),
    Domain(Pattern),
    GfwList,
    AdBlockList,
}

impl FromStr for TrafficMatchRule {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut splits = s.split(':');
        let t = splits
            .next()
            .ok_or_else(|| anyhow::anyhow!("Invalid rule: {s}"))?;
        let v = splits.next();
        if t.eq_ignore_ascii_case("geoip") {
            Ok(Self::GeoIP(
                v.and_then(|d| d.parse().ok())
                    .ok_or_else(|| anyhow!("Invalid geoip"))?,
            ))
        } else if t.eq_ignore_ascii_case("network") {
            Ok(Self::Network(
                v.and_then(|d| d.parse().ok())
                    .ok_or_else(|| anyhow!("Invalid networks"))?,
            ))
        } else if t.eq_ignore_ascii_case("gfwlist") {
            Ok(Self::GfwList)
        } else if t.eq_ignore_ascii_case("adblocklist") {
            Ok(Self::AdBlockList)
        } else if t.eq_ignore_ascii_case("domain") {
            Ok(Self::Domain(
                v.and_then(|d| d.parse().ok())
                    .ok_or_else(|| anyhow!("Invalid domain pattern"))?,
            ))
        } else {
            Err(anyhow::anyhow!("Invalid rule: {s}"))
        }
    }
}

impl Display for TrafficMatchRule {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GeoIP(c) => f.write_fmt(format_args!("geoip:{c}")),
            Self::Network(n) => f.write_fmt(format_args!("network:{n}")),
            Self::Domain(p) => f.write_fmt(format_args!("domain:{p}")),
            Self::GfwList => f.write_str("gfwlist"),
            Self::AdBlockList => f.write_str("adblocklist"),
        }
    }
}

impl TrafficMatchRule {
    pub fn calc_match_score(
        &self,
        country_code: Option<CountryCode>,
        ip: Option<IpAddr>,
        addr: &Address,
    ) -> usize {
        match (self, country_code, ip, addr) {
            (Self::GeoIP(cc), Some(c), _, _) if cc == &c => 10,
            (Self::Network(network), _, Some(ip), _) if network.contains(ip) => 20,
            (Self::GfwList, _, _, Address::Name { .. }) if gfw_list_engine().matches(addr) => 15,
            (Self::AdBlockList, _, _, _) if adblock_list_engine().matches(addr) => 20,
            (Self::Domain(p), _, _, Address::Name { host, .. }) if p.matches(host.as_ref()) => 20,
            _ => 0,
        }
    }
}

const fn default_upstream_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum UpstreamProtocol {
    #[serde(rename = "udpman")]
    UdpMan(udpman::UdpMan),

    #[serde(rename = "tcpman")]
    TcpMan(tcpman::TcpMan),

    #[serde(rename = "socks5")]
    Socks5(socks5::Socks5),

    #[serde(rename = "direct")]
    Direct(direct::Direct),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpstreamConfig {
    pub protocol: UpstreamProtocol,
    #[serde(default)]
    pub accept: Vec<TrafficMatchRule>,
    #[serde(default)]
    pub reject: Vec<TrafficMatchRule>,
    #[serde(default)]
    pub priority: u16,
    #[serde(default = "default_upstream_enabled")]
    pub enabled: bool,
}

impl UpstreamConfig {
    fn calc_score(&self, target: &Address, country_code: Option<CountryCode>) -> usize {
        if !self.enabled {
            return 0;
        }

        let ip = match target {
            Address::IP(a) => Some(a.ip()),
            _ => None,
        };

        let mut score = 5;
        if !self.accept.is_empty() {
            score = self.accept.iter().fold(0usize, |acc, item| {
                usize::max(item.calc_match_score(country_code, ip, &target), acc)
            });

            if score == 0 {
                // No match for accept rules
                return 0;
            }
        }

        if !self.reject.is_empty() {
            let has_reject_rule = self
                .reject
                .iter()
                .find(|r| r.calc_match_score(country_code, ip, &target) > 0)
                .is_some();
            if has_reject_rule {
                return 0;
            }
        }

        score += (u16::MAX - self.priority) as usize;
        score
    }
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
    pub udp_tproxy_address: Option<SocketAddr>,

    #[serde(default)]
    pub set_router_rules: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            socks5_address: default_socks5_address(),
            socks5_udp_host: default_socks5_udp_host(),
            upstreams: Default::default(),
            fwmark: None,
            udp_tproxy_address: None,
            set_router_rules: false,
        }
    }
}

impl ClientConfig {
    fn calc_last_visit_score(stats: &ClientStatistics, upstream_name: &String) -> usize {
        (match stats.upstreams.get(upstream_name) {
            Some(stat) => {
                let last = stat.last_activity.get() as u64;
                let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
                now.checked_sub(last)
                    .unwrap_or(0)
                    .try_into()
                    .unwrap_or(u16::MAX)
            }
            _ => u16::MAX,
        }) as usize
    }

    // Sorted by score MIN -> MAX
    pub fn find_best_upstream(
        &self,
        t: TrafficType,
        stats: &ClientStatistics,
        target: &Address,
    ) -> Vec<(&str, &UpstreamConfig)> {
        let country_code = match target {
            Address::IP(addr) => find_geoip(&addr.ip()),
            _ => None,
        };

        if let Some(c) = &country_code {
            log::debug!("Got country code {c} for {target}");
        }

        let mut upstreams: Vec<(&str, &UpstreamConfig, usize)> = {
            // Find suitable upstreams first
            self.upstreams
                .iter()
                .filter_map(|(n, c)| {
                    if !c.protocol.supports(t) {
                        return None;
                    }

                    match c.calc_score(target, country_code) {
                        0 => None,
                        score => {
                            Some((n.as_str(), c, score + Self::calc_last_visit_score(stats, n)))
                        }
                    }
                })
                .filter(|(_, _, score)| *score > 0)
                .collect()
        };

        upstreams.sort_by_key(|(_, _, score)| *score);
        upstreams.into_iter().map(|(n, c, _)| (n, c)).collect()
    }
}

#[async_trait]
impl Protocol for UpstreamProtocol {
    fn supports(&self, traffic_type: TrafficType) -> bool {
        match self {
            UpstreamProtocol::UdpMan(p) => p.supports(traffic_type),
            UpstreamProtocol::TcpMan(p) => p.supports(traffic_type),
            UpstreamProtocol::Direct(p) => p.supports(traffic_type),
            UpstreamProtocol::Socks5(p) => p.supports(traffic_type),
        }
    }

    async fn new_stream(
        &self,
        dst: &Address<'_>,
        initial_data: Option<&[u8]>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Box<dyn AsyncStream>> {
        match self {
            UpstreamProtocol::UdpMan(p) => p.new_stream(dst, initial_data, stats, fwmark).await,
            UpstreamProtocol::TcpMan(p) => p.new_stream(dst, initial_data, stats, fwmark).await,
            UpstreamProtocol::Direct(p) => p.new_stream(dst, initial_data, stats, fwmark).await,
            UpstreamProtocol::Socks5(p) => p.new_stream(dst, initial_data, stats, fwmark).await,
        }
    }

    async fn new_datagram(
        &self,
        dst: &Address<'_>,
        initial_data: Bytes,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        match self {
            UpstreamProtocol::UdpMan(p) => p.new_datagram(dst, initial_data, stats, fwmark).await,
            UpstreamProtocol::TcpMan(p) => p.new_datagram(dst, initial_data, stats, fwmark).await,
            UpstreamProtocol::Direct(p) => p.new_datagram(dst, initial_data, stats, fwmark).await,
            UpstreamProtocol::Socks5(p) => p.new_datagram(dst, initial_data, stats, fwmark).await,
        }
    }
}
