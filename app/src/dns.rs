use std::net::IpAddr;

#[derive(Default)]
pub struct DnsResultCache {}

impl DnsResultCache {
    pub fn find_domain(&self, addr: IpAddr) -> Option<String> {
        None
    }

    pub fn find_ip(&self, domain: &str) -> Option<IpAddr> {
        None
    }

    pub async fn cache(&self, domain: &str, ip: Vec<IpAddr>) {}
}
