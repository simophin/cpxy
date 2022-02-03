use anyhow::anyhow;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use ipnetwork::IpNetwork;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::abp::{matches_adblock_list, matches_gfw_list};
use crate::client::ClientStatistics;
use crate::geoip::{find_geoip, CountryCode};
use crate::socks5::Address;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum UpstreamRule {
    GeoIP(CountryCode),
    Network(IpNetwork),
    GfwList,
    AdBlockList,
}

struct UpstreamRuleVisitor;

impl<'de> Visitor<'de> for UpstreamRuleVisitor {
    type Value = UpstreamRule;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Upstream rule")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        v.parse::<Self::Value>()
            .map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}

impl<'de> Deserialize<'de> for UpstreamRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(UpstreamRuleVisitor)
    }
}

impl Serialize for UpstreamRule {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

impl FromStr for UpstreamRule {
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
        } else if t.eq_ignore_ascii_case("adblock") {
            Ok(Self::AdBlockList)
        } else {
            Err(anyhow::anyhow!("Invalid rule: {s}"))
        }
    }
}

impl Display for UpstreamRule {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GeoIP(c) => f.write_fmt(format_args!("geoip:{c}")),
            Self::Network(n) => f.write_fmt(format_args!("network:{n}")),
            Self::GfwList => f.write_str("gfwlist"),
            Self::AdBlockList => f.write_str("adblocklist"),
        }
    }
}

impl UpstreamRule {
    pub fn matches(
        &self,
        country_code: Option<CountryCode>,
        ip: Option<IpAddr>,
        addr: &Address,
    ) -> bool {
        match (self, country_code, ip, addr) {
            (Self::GeoIP(cc), Some(c), _, _) => cc == &c,
            (Self::Network(network), _, Some(ip), _) => network.contains(ip),
            (Self::GfwList, _, _, Address::Name { .. }) => matches_gfw_list(addr),
            (Self::AdBlockList, _, _, _) => matches_adblock_list(addr),
            _ => false,
        }
    }

    fn matches_any<'a>(
        mut rules: impl Iterator<Item = &'a UpstreamRule>,
        country_code: Option<CountryCode>,
        ip: Option<IpAddr>,
        addr: &Address,
    ) -> bool {
        rules
            .find(|r| r.matches(country_code.clone(), ip.clone(), addr))
            .is_some()
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct UpstreamConfig {
    pub address: Address,
    #[serde(default)]
    pub accept: Vec<UpstreamRule>,
    #[serde(default)]
    pub reject: Vec<UpstreamRule>,
    #[serde(default)]
    pub priority: u16,
}

impl UpstreamConfig {
    fn matches(&self, target: &Address) -> bool {
        let ip = match target {
            Address::IP(a) => Some(a.ip()),
            _ => None,
        };

        let country_code = ip.as_ref().and_then(|v| find_geoip(v));

        (self.accept.is_empty()
            || UpstreamRule::matches_any(self.accept.iter(), country_code, ip, target))
            && (self.reject.is_empty()
                || !UpstreamRule::matches_any(self.reject.iter(), country_code, ip, target))
    }
}

fn default_socks5_udp_host() -> String {
    "0.0.0.0".to_string()
}

fn default_socks5_address() -> Address {
    "127.0.0.1:5000".parse().unwrap()
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientConfig {
    pub upstreams: HashMap<String, UpstreamConfig>,

    #[serde(default = "default_socks5_address")]
    pub socks5_address: Address,

    #[serde(default = "default_socks5_udp_host")]
    pub socks5_udp_host: String,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            upstreams: Default::default(),
            socks5_address: default_socks5_address(),
            socks5_udp_host: default_socks5_udp_host(),
        }
    }
}

impl ClientConfig {
    fn calc_last_visit_score(stats: &ClientStatistics, upstream_name: &String) -> usize {
        (match stats.upstreams.get(upstream_name) {
            Some(stat) => {
                let last = stat.last_activity.load(Ordering::Relaxed);
                let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
                now.checked_sub(last)
                    .unwrap_or(0)
                    .try_into()
                    .unwrap_or(u16::MAX)
            }
            _ => u16::MAX,
        }) as usize
    }

    pub fn find_best_upstream(
        &self,
        stats: Arc<ClientStatistics>,
        target: &Address,
    ) -> Option<(&str, &UpstreamConfig)> {
        let mut upstreams: Vec<(&str, &UpstreamConfig, usize)> = {
            // Find suitable upstreams first
            self.upstreams
                .iter()
                .filter(|(_, c)| c.matches(target))
                .map(|(n, c)| {
                    (
                        n.as_str(),
                        c,
                        (u16::MAX - c.priority) as usize
                            + Self::calc_last_visit_score(stats.as_ref(), n),
                    )
                })
                .collect()
        };

        upstreams.sort_by_key(|(_, _, score)| *score);
        let result = upstreams.last().map(|(n, c, _)| (*n, *c));

        if let Some((name, _)) = result.as_ref() {
            match stats.upstreams.get(*name) {
                Some(stat) => stat
                    .last_activity
                    .store(UNIX_EPOCH.elapsed().unwrap().as_secs(), Ordering::Relaxed),
                _ => {}
            }
        }

        result
    }
}
