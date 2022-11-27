use std::{
    borrow::Cow,
    collections::HashMap,
    fmt::Debug,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::Arc,
};

use anyhow::{bail, Context};
use clap::{Parser, ValueEnum};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};

use crate::sni::{extract_http_host_header, extract_ssl_sni_host};
use crate::{
    abp::{adblock_list_engine, gfw_list_engine, ABPEngine},
    dns::dns_get_host_names,
    geoip::CountryCode,
    pattern::Pattern,
    socks5::Address,
};

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum HostMatch {
    Pattern(Pattern),
    HostList(&'static ABPEngine),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RuleDestination {
    GeoIP(CountryCode),
    Network(IpNetwork),
    Port(u16),
    Domain(HostMatch),
    DnsHost(HostMatch),
}

#[derive(Debug, ValueEnum, Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
pub enum RuleProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
enum RuleAction {
    Proxy(Arc<str>),
    ProxyGroup(Arc<str>),
    Reject,
    Jump(Arc<str>),
    Return,
}

enum TableExecuteResult<'a> {
    Proxy(&'a str),
    ProxyGroup(&'a str),
    Reject,
    Return,
}

#[derive(Debug, Parser, PartialEq, Eq, Clone)]
pub struct Rule {
    #[clap(short)]
    dest: Vec<RuleDestination>,
    #[clap(short)]
    proto: Option<RuleProtocol>,
    #[clap(short)]
    action: RuleAction,
}

impl FromStr for RuleDestination {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, args) = s.split_once(':').unwrap_or((s, ""));
        match name {
            "geoip" => {
                Ok(Self::GeoIP(args.parse().with_context(|| {
                    format!("Parsing args into country code: {args}")
                })?))
            }
            "network" => {
                Ok(Self::Network(args.parse().with_context(|| {
                    format!("Parsing args into network: {args}")
                })?))
            }
            "port" => {
                Ok(Self::Port(args.parse().with_context(|| {
                    format!("Parsing args into port: {args}")
                })?))
            }
            "domain" => {
                Ok(Self::Domain(args.parse().with_context(|| {
                    format!("Parsing args into domain: {args}")
                })?))
            }
            "dnshost" => {
                Ok(Self::DnsHost(args.parse().with_context(|| {
                    format!("Parsing args into dnshost: {args}")
                })?))
            }
            _ => bail!("Unknown rule: {s}"),
        }
    }
}

impl FromStr for RuleAction {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut splits = s.split(':');
        match (splits.next(), splits.next()) {
            (Some(n), Some(v)) if n.eq_ignore_ascii_case("proxy") => Ok(Self::Proxy(v.into())),
            (Some(n), Some(v)) if n.eq_ignore_ascii_case("proxygroup") => {
                Ok(Self::ProxyGroup(v.into()))
            }
            (Some(n), None) if n.eq_ignore_ascii_case("reject") => Ok(Self::Reject),
            (Some(n), Some(table_name)) if n.eq_ignore_ascii_case("jump") => {
                Ok(Self::Jump(table_name.into()))
            }
            (Some(n), None) if n.eq_ignore_ascii_case("return") => Ok(Self::Return),
            _ => {
                bail!("Unknown rule action {s}")
            }
        }
    }
}

impl Rule {
    pub fn parse_rules(s: &str) -> anyhow::Result<HashMap<String, Vec<Rule>>> {
        let mut rulemap = HashMap::<String, Vec<Rule>>::new();
        let mut last_name = None;

        for line in s.split('\n') {
            let line = match line.trim() {
                v if v.is_empty() => continue,
                v => v,
            };

            if line.starts_with("#") {
                continue;
            }

            if line.ends_with(":") {
                let name = &line[..line.len() - 1];
                if name.is_empty() {
                    bail!("Name line must not be empty");
                }

                last_name.replace(name);
                continue;
            }

            let name = last_name
                .as_ref()
                .context("Expecting a table name before rules")?;

            let rule = Rule::try_parse_from(line.split_ascii_whitespace())
                .with_context(|| format!("Parsing rule \"{line}\""))?;
            match rulemap.get_mut(*name) {
                Some(rules) => rules.push(rule),
                None => {
                    rulemap.insert(name.to_string(), vec![rule]);
                }
            }
        }

        Ok(rulemap)
    }
}

#[derive(Eq, Default, Clone)]
pub struct RuleString {
    s: String,
    rules: HashMap<String, Vec<Rule>>,
}

#[derive(PartialEq, Eq, Debug)]
pub enum RuleExecutionResult<'a> {
    Proxy(&'a str),
    ProxyGroup(&'a str),
    Reject,
}

#[derive(Debug)]
pub enum PacketDestination<'a> {
    IP {
        addr: SocketAddr,
        country_code: Option<CountryCode>,
        resolved_host: Vec<Arc<str>>,
    },
    Domain {
        hostname: &'a str,
        port: u16,
        resolved_ips: Vec<(Option<CountryCode>, IpAddr)>,
    },
}

impl RuleString {
    fn execute_table<'a>(
        &'a self,
        level: usize,
        table_name: &str,
        target: &PacketDestination<'_>,
        proto: RuleProtocol,
        initial_data: Option<&[u8]>,
    ) -> Option<TableExecuteResult<'a>> {
        if level > 10 {
            log::error!("Too many level of table executions");
            return None;
        }

        let table_rules = match self.rules.get(table_name) {
            Some(v) => v,
            None => {
                log::warn!("Table named {table_name} doesn't exist");
                return None;
            }
        };

        for rule in table_rules {
            let mut matches_dest = true;
            for dest in &rule.dest {
                // Match dest
                matches_dest &= dest.matches(target, initial_data);

                if !matches_dest {
                    break;
                }
            }

            if !matches_dest {
                continue;
            }

            // Match protocol
            if matches!(rule.proto, Some(p) if p != proto) {
                continue;
            }

            log::debug!("Matched {rule:?}");
            match &rule.action {
                RuleAction::Jump(table_name) => {
                    match self.execute_table(
                        level + 1,
                        table_name.as_ref(),
                        target,
                        proto,
                        initial_data.clone(),
                    ) {
                        Some(TableExecuteResult::Return) => {}
                        v => return v,
                    };
                }
                RuleAction::Proxy(name) => return Some(TableExecuteResult::Proxy(name.as_ref())),
                RuleAction::ProxyGroup(name) => {
                    return Some(TableExecuteResult::ProxyGroup(name.as_ref()));
                }
                RuleAction::Reject => return Some(TableExecuteResult::Reject),
                RuleAction::Return => return Some(TableExecuteResult::Return),
            }
        }
        None
    }

    pub fn execute_rules<'a>(
        &'a self,
        target: &PacketDestination<'_>,
        proto: RuleProtocol,
        initial_data: Option<&[u8]>,
    ) -> anyhow::Result<Option<RuleExecutionResult<'a>>> {
        // Start from main table
        match self.execute_table(0, "main", target, proto, initial_data) {
            Some(TableExecuteResult::Proxy(name)) => Ok(Some(RuleExecutionResult::Proxy(name))),
            Some(TableExecuteResult::ProxyGroup(name)) => {
                Ok(Some(RuleExecutionResult::ProxyGroup(name)))
            }
            Some(TableExecuteResult::Reject) => Ok(Some(RuleExecutionResult::Reject)),
            None | Some(TableExecuteResult::Return) => Ok(None),
        }
    }
}

impl RuleDestination {
    fn matches(&self, target: &PacketDestination<'_>, initial_data: Option<&[u8]>) -> bool {
        match (self, target) {
            (RuleDestination::GeoIP(c), PacketDestination::IP { country_code, .. }) => {
                country_code.as_ref() == Some(c)
            }
            (RuleDestination::GeoIP(c), PacketDestination::Domain { resolved_ips, .. }) => {
                resolved_ips
                    .iter()
                    .find(|(code, _)| code.as_ref() == Some(c))
                    .is_some()
            }
            (RuleDestination::Network(n), PacketDestination::IP { addr, .. }) => {
                n.contains(addr.ip())
            }
            (RuleDestination::Network(n), PacketDestination::Domain { resolved_ips, .. }) => {
                resolved_ips
                    .iter()
                    .find(|(_, addr)| n.contains(*addr))
                    .is_some()
            }
            (RuleDestination::Domain(p), dst) => Self::domain_matches(p, dst, initial_data),
            (RuleDestination::Port(p), pd) => *p == pd.port(),
            (RuleDestination::DnsHost(p), pd) => {
                pd.port() == 53
                    && initial_data.is_some()
                    && dns_get_host_names(initial_data.unwrap())
                        .and_then(|mut host_names| host_names.position(|h| p.matches(h.as_str())))
                        .is_some()
            }
        }
    }

    fn domain_matches(
        host_match: &HostMatch,
        target: &PacketDestination<'_>,
        initial_data: Option<&[u8]>,
    ) -> bool {
        // Do we have a definite domain name?
        if let PacketDestination::Domain { hostname, .. } = target {
            return host_match.matches(*hostname);
        }

        // Can we extract the real host from HTTP/HTTPs?
        if let Some(initial_data) = initial_data {
            let header = match target.port() {
                80 => extract_http_host_header(initial_data),
                443 => extract_ssl_sni_host(initial_data),
                _ => None,
            };

            if let Some(host) = header {
                return host_match.matches(host);
            }
        }

        // Now we have to trust the "resolved DOMAIN", this is the last resort and less accurate
        if let PacketDestination::IP { resolved_host, .. } = target {
            return resolved_host
                .iter()
                .filter(|h| host_match.matches(h.as_ref()))
                .next()
                .is_some();
        }

        false
    }
}

impl<'a> PacketDestination<'a> {
    pub fn port(&self) -> u16 {
        match self {
            Self::Domain { port, .. } => *port,
            Self::IP { addr, .. } => addr.port(),
        }
    }
}

impl HostMatch {
    fn matches(&self, s: &str) -> bool {
        match self {
            HostMatch::Pattern(p) => p.matches(s),
            HostMatch::HostList(engine) => engine.matches(&Address::Name {
                host: Cow::Borrowed(s),
                port: 80,
            }),
        }
    }
}

impl FromStr for HostMatch {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut splits = s.split(':');
        Ok(match (splits.next(), splits.next()) {
            (Some("list"), Some("gfw")) => Self::HostList(gfw_list_engine()),
            (Some("list"), Some("adblock")) => Self::HostList(adblock_list_engine()),
            (Some("matches"), Some(p)) => Self::Pattern(p.parse()?),
            _ => bail!(
                "Invalid host match string: {s}. Expect: list:gfw, list:adblock or matches:pattern"
            ),
        })
    }
}

impl Debug for RuleString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.s.fmt(f)
    }
}

impl PartialEq for RuleString {
    fn eq(&self, other: &Self) -> bool {
        self.s.eq(&other.s)
    }
}

impl Serialize for RuleString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.s.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RuleString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <String as Deserialize<'_>>::deserialize(deserializer)?;
        let rules = Rule::parse_rules(&s).map_err(|e| serde::de::Error::custom(e))?;
        Ok(Self { s, rules })
    }
}

#[cfg(test)]
mod tests {
    use maplit::hashmap;

    use super::*;

    #[test]
    fn rule_parsing_works() {
        let rules = "\
        main:\n\
            test -d domain:list:gfw -p tcp -a proxy:proxy1\n\
            test -d domain:list:adblock -p tcp -a proxy:proxy1\n\
            test -d geoip:cn -d geoip:us -p udp -a reject\n\
            test -d geoip:nz -a jump:nz\n\
            test -a reject\n\
        nz:\n\
            test -p udp -a return\n\
            test -a proxygroup:group\n\
        ";

        let expect = hashmap! {
            "main".to_string() => vec![
                Rule {
                    dest: vec![RuleDestination::Domain(HostMatch::HostList(gfw_list_engine()))],
                    proto: Some(RuleProtocol::Tcp),
                    action: RuleAction::Proxy("proxy1".into()),
                },
                Rule {
                    dest: vec![RuleDestination::Domain(HostMatch::HostList(adblock_list_engine()))],
                    proto: Some(RuleProtocol::Tcp),
                    action: RuleAction::Proxy("proxy1".into()),
                },
                Rule {
                    dest: vec![
                        RuleDestination::GeoIP("CN".parse().unwrap()),
                        RuleDestination::GeoIP("us".parse().unwrap()),
                    ],
                    proto: Some(RuleProtocol::Udp),
                    action: RuleAction::Reject,
                },
                Rule {
                    dest: vec![RuleDestination::GeoIP("nz".parse().unwrap())],
                    proto: None,
                    action: RuleAction::Jump("nz".into())
                },
                Rule {
                    dest: vec![],
                    proto: None,
                    action: RuleAction::Reject
                }
            ],
            "nz".to_string() => vec![
                Rule {
                    dest: vec![],
                    proto: Some(RuleProtocol::Udp),
                    action: RuleAction::Return,
                },
                Rule {
                    dest: vec![],
                    proto: None,
                    action: RuleAction::ProxyGroup("group".into()),
                },
            ]
        };

        let rulemap = Rule::parse_rules(rules).expect("To parse rules");
        assert_eq!(rulemap, expect);

        let rules = RuleString {
            rules: rulemap,
            s: rules.to_string(),
        };

        let action = rules
            .execute_rules(
                &PacketDestination::IP {
                    addr: "1.2.3.4:443".parse().unwrap(),
                    country_code: Some("nz".parse().unwrap()),
                    resolved_host: Default::default(),
                },
                RuleProtocol::Tcp,
                None,
            )
            .unwrap();

        assert_eq!(
            action,
            Some(RuleExecutionResult::ProxyGroup("group".into()))
        );
    }
}
