use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::UNIX_EPOCH;

use crate::client::ClientStatistics;
use crate::dns::DnsCache;
use crate::geoip::find_geoip;
use crate::protocol::{
    direct, firetcp, socks5, tcpman, udpman, AsyncStream, BoxedSink, BoxedStream, Protocol, Stats,
    TrafficType,
};
use crate::rule::{PacketDestination, RuleExecutionResult, RuleProtocol, RuleString};
use crate::socks5::Address;

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

    #[serde(rename = "firetcp")]
    FireTcp(firetcp::FireTcp),
}

pub const fn default_upstream_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpstreamConfig {
    pub protocol: UpstreamProtocol,
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
    pub udp_tproxy_address: Option<SocketAddr>,

    #[serde(default)]
    pub traffic_rules: RuleString,

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
            traffic_rules: Default::default(),
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
        initial_data: Option<&[u8]>,
    ) -> anyhow::Result<Vec<(&str, &UpstreamConfig)>> {
        let pkt_dst = match target {
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

        let action = self.traffic_rules.execute_rules(
            &pkt_dst,
            match t {
                TrafficType::Datagram => RuleProtocol::Udp,
                TrafficType::Stream => RuleProtocol::Tcp,
            },
            initial_data,
        )?;

        let mut upstreams: Vec<(&str, &UpstreamConfig, usize)> = match action {
            None => self
                .upstreams
                .iter()
                .filter_map(|(n, c)| {
                    if !c.protocol.supports(t) {
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
                    if !c.protocol.supports(t) || !c.enabled {
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

#[async_trait]
impl Protocol for UpstreamProtocol {
    fn supports(&self, traffic_type: TrafficType) -> bool {
        match self {
            UpstreamProtocol::UdpMan(p) => p.supports(traffic_type),
            UpstreamProtocol::TcpMan(p) => p.supports(traffic_type),
            UpstreamProtocol::Direct(p) => p.supports(traffic_type),
            UpstreamProtocol::Socks5(p) => p.supports(traffic_type),
            UpstreamProtocol::FireTcp(p) => p.supports(traffic_type),
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
            UpstreamProtocol::FireTcp(p) => p.new_stream(dst, initial_data, stats, fwmark).await,
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
            UpstreamProtocol::FireTcp(p) => p.new_datagram(dst, initial_data, stats, fwmark).await,
        }
    }
}
