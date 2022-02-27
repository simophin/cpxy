use std::net::IpAddr;

#[derive(Default)]
pub struct DnsResultCache {}

impl DnsResultCache {
    pub fn find_domain(&self, _: IpAddr) -> Option<String> {
        None
    }

    pub fn find_ip(&self, _: &str) -> Option<IpAddr> {
        None
    }

    pub async fn cache(&self, _: &str, _: Vec<IpAddr>) {}
}
