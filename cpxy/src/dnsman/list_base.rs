use std::{net::SocketAddr, path::Path};

use async_trait::async_trait;

pub struct Rule {
    pub name: String,
    pub list_uri: String,
    pub addr: SocketAddr,
}

pub struct ListBasedDomainListProvider {
    rules: Vec<Rule>,
    fallback: Option<Rule>,
}

#[async_trait]
impl super::DomainListProvider for ListBasedDomainListProvider {
    async fn dns_server_for_domain(&self, domain: &str) -> Option<SocketAddr> {
        todo!()
    }
}

impl ListBasedDomainListProvider {
    pub fn new(rules: Vec<Rule>, fallback: Option<Rule>) -> Self {
        Self { rules, fallback }
    }

    async fn download_list(base_cache_path: &Path, uri: &str) -> anyhow::Result<Vec<String>> {
        todo!()
    }
}