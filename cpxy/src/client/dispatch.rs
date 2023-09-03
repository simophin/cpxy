use std::{
    borrow::Cow,
    cell::OnceCell,
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Once},
};

use async_trait::async_trait;
use dashmap::DashMap;
use either::Either;
use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use smallvec::SmallVec;

use crate::{
    domain_list::DomainListRepository,
    geoip::CountryCode,
    protocol::{DynamicProtocol, Protocol, ProtocolReporter, ProxyRequest},
    regex::Regex,
    sni::{extract_http_host_header, extract_ssl_sni_host},
};

use super::{rule::ExecutionContext, settings::UpstreamSettings};

#[derive(Clone)]
pub struct DispatchProtocol {
    stream: Arc<RwLock<Vec<UpstreamSettings>>>,
    stats: Arc<DashMap<String, Arc<dyn ProtocolReporter>>>,
}

#[async_trait]
impl Protocol for DispatchProtocol {
    type ClientStream = <DynamicProtocol as Protocol>::ClientStream;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        reporter: &Arc<dyn ProtocolReporter>,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::ClientStream> {
        todo!()
    }
}

impl DispatchProtocol {
    fn find_upstreams(
        &self,
        req: &ProxyRequest,
    ) -> impl IntoIterator<Item = (Arc<DynamicProtocol>, Arc<dyn ProtocolReporter>)> {
        let mut result: SmallVec<[(Arc<DynamicProtocol>, Arc<dyn ProtocolReporter>); 3]> =
            Default::default();

        result
    }
}

pub trait DnsCacheLookup {
    fn lookup(&self, host: &str) -> Option<IpAddr>;
    fn reverse(&self, ip: IpAddr) -> Option<String>;
}

pub trait IpCountryLookup {
    fn lookup(&self, ip: IpAddr) -> Option<CountryCode>;
}

struct DispatchProgramContext<'a> {
    req: &'a ProxyRequest,
    dns_cache_lookup: Box<dyn DnsCacheLookup>,
    ip_country_lookup: Box<dyn IpCountryLookup>,
    domain_list_repository: &'a DomainListRepository,

    domain: OnceCell<Option<Cow<'a, str>>>,
    port: OnceCell<String>,
    ip: OnceCell<Option<String>>,
    ip_country: OnceCell<Option<CountryCode>>,

    os_timezone: OnceCell<Option<String>>,
}

impl<'a> DispatchProgramContext<'a> {
    fn domain(&self) -> Option<&str> {
        self.domain
            .get_or_init(|| {
                // Is it in the request?
                if let Some(host) = self.req.dst.domain() {
                    return Some(Cow::Borrowed(host));
                }

                // Is it in the initial data?
                if let Some(data) = &self.req.initial_data {
                    if let Some(host) = extract_http_host_header(data) {
                        return Some(Cow::Borrowed(host));
                    }

                    if let Some(host) = extract_ssl_sni_host(data) {
                        return Some(Cow::Borrowed(host));
                    }
                }

                // Is it in the DNS cache?
                if let Some(ip) = self.req.dst.ip() {
                    if let Some(host) = self.dns_cache_lookup.reverse(ip) {
                        return Some(Cow::Owned(host));
                    }
                }

                None
            })
            .as_ref()
            .map(|s| s.as_ref())
    }

    fn port_str(&self) -> Option<&str> {
        Some(
            self.port
                .get_or_init(|| self.req.dst.port().to_string())
                .as_str(),
        )
    }

    fn ip(&self) -> Option<IpAddr> {
        if let Some(ip) = self.req.dst.ip() {
            return Some(ip);
        }

        if let Some(host) = self.domain() {
            if let Some(ip) = self.dns_cache_lookup.lookup(host) {
                return Some(ip);
            }
        }

        None
    }

    fn ip_str(&self) -> Option<&str> {
        self.ip
            .get_or_init(|| self.ip().map(|s| s.to_string()))
            .as_ref()
            .map(|s| s.as_str())
    }

    fn ip_country(&self) -> Option<&str> {
        self.ip_country
            .get_or_init(|| self.ip().and_then(|ip| self.ip_country_lookup.lookup(ip)))
            .as_ref()
            .map(|s| s.as_ref())
    }

    fn os_timezone(&self) -> Option<&str> {
        self.os_timezone
            .get_or_init(|| iana_time_zone::get_timezone().ok())
            .as_ref()
            .map(|s| s.as_str())
    }
}

impl<'a> ExecutionContext for DispatchProgramContext<'a> {
    fn get_property(&self, key: &str) -> Option<&str> {
        let mut splits = key.splitn(2, '.');
        match (splits.next()?, splits.next()?) {
            ("req", "domain") => self.domain(),
            ("req", "port") => self.port_str(),
            ("req", "ip") => self.ip_str(),
            ("req", "ip_country") => self.ip_country(),
            ("os", "timezone") => self.os_timezone(),
            _ => None,
        }
    }

    fn check_value_in(&self, key: &str, list_name: &str) -> bool {
        match (key, list_name) {
            ("req.domain_country", _) => {
                let domain = match self.domain() {
                    Some(d) => d,
                    None => return false,
                };

                let country_code: CountryCode = match list_name.parse() {
                    Ok(c) => c,
                    Err(_) => return false,
                };

                self.domain_list_repository.find_country_recursive(domain) == Some(country_code)
            }

            _ => false,
        }
    }

    fn available_properties(&self) -> &[&Regex] {
        todo!()
    }

    fn available_list_operations(&self) -> &[(&Regex, &Regex)] {
        todo!()
    }
}
