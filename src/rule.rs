use std::{collections::HashMap, fmt::Debug, str::FromStr, sync::Arc};

use anyhow::{bail, Context};
use clap::{ArgEnum, Parser};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};

use crate::{
    abp::{adblock_list_engine, gfw_list_engine},
    geoip::CountryCode,
    pattern::Pattern,
    socks5::Address,
};

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RuleDestination {
    GfwList,
    AdBlockList,
    GeoIP(CountryCode),
    Network(IpNetwork),
    Port(u16),
    Domain(Pattern),
}

#[derive(Debug, ArgEnum, Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
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
    #[clap(short, parse(try_from_str))]
    dest: Vec<RuleDestination>,
    #[clap(arg_enum, short)]
    proto: Option<RuleProtocol>,
    #[clap(short, parse(try_from_str))]
    action: RuleAction,
}

impl FromStr for RuleDestination {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut splits = s.split(':');
        match (splits.next(), splits.next()) {
            (Some(a), None) if a.eq_ignore_ascii_case("gfwlist") => Ok(Self::GfwList),
            (Some(a), None) if a.eq_ignore_ascii_case("adblocklist") => Ok(Self::AdBlockList),
            (Some(a), Some(code)) if a.eq_ignore_ascii_case("geoip") => {
                let c: CountryCode = code
                    .parse()
                    .with_context(|| format!("Parsing country code: {code}"))?;
                Ok(Self::GeoIP(c))
            }
            (Some(a), Some(network)) if a.eq_ignore_ascii_case("network") => {
                let n: IpNetwork = network
                    .parse()
                    .with_context(|| format!("Parsing network: {network}"))?;
                Ok(Self::Network(n))
            }
            (Some(a), Some(p)) if a.eq_ignore_ascii_case("port") => {
                let port: u16 = p.parse().context("Parsing port")?;
                Ok(Self::Port(port))
            }
            (Some(a), Some(p)) if a.eq_ignore_ascii_case("domain") => {
                Ok(Self::Domain(p.parse().context("Parsing domain pattern")?))
            }
            _ => bail!("Invalid rule: {s}"),
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

impl RuleString {
    fn execute_table<'a>(
        &'a self,
        level: usize,
        table_name: &str,
        target: &Address<'_>,
        proto: RuleProtocol,
        country: Option<CountryCode>,
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
                matches_dest &= match dest {
                    RuleDestination::GfwList => gfw_list_engine().matches(target),
                    RuleDestination::AdBlockList => adblock_list_engine().matches(target),
                    RuleDestination::GeoIP(c) => country == Some(*c),
                    RuleDestination::Network(n) => match target {
                        Address::IP(ip) => n.contains(ip.ip()),
                        _ => false,
                    },
                    RuleDestination::Domain(p) => match target {
                        Address::Name { host, .. } => p.matches(host.as_ref()),
                        _ => false,
                    },
                    RuleDestination::Port(p) => *p == target.get_port(),
                };

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
                    match self.execute_table(level + 1, table_name.as_ref(), target, proto, country)
                    {
                        Some(TableExecuteResult::Return) => {}
                        v => return v,
                    };
                }
                RuleAction::Proxy(name) => return Some(TableExecuteResult::Proxy(name.as_ref())),
                RuleAction::ProxyGroup(name) => {
                    return Some(TableExecuteResult::ProxyGroup(name.as_ref()))
                }
                RuleAction::Reject => return Some(TableExecuteResult::Reject),
                RuleAction::Return => return Some(TableExecuteResult::Return),
            }
        }
        None
    }

    pub fn execute_rules<'a>(
        &'a self,
        target: &Address<'_>,
        proto: RuleProtocol,
        country: Option<CountryCode>,
    ) -> anyhow::Result<Option<RuleExecutionResult<'a>>> {
        // Start from main table
        match self.execute_table(0, "main", target, proto, country) {
            Some(TableExecuteResult::Proxy(name)) => Ok(Some(RuleExecutionResult::Proxy(name))),
            Some(TableExecuteResult::ProxyGroup(name)) => {
                Ok(Some(RuleExecutionResult::ProxyGroup(name)))
            }
            Some(TableExecuteResult::Reject) => Ok(Some(RuleExecutionResult::Reject)),
            None | Some(TableExecuteResult::Return) => Ok(None),
        }
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
            test -d gfwlist -p tcp -a proxy:proxy1\n\
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
                    dest: vec![RuleDestination::GfwList],
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
                &Default::default(),
                RuleProtocol::Tcp,
                Some("nz".parse().unwrap()),
            )
            .unwrap();

        assert_eq!(
            action,
            Some(RuleExecutionResult::ProxyGroup("group".into()))
        );
    }
}
