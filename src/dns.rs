use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use async_io::Timer;
use dns_parser::{Packet, RData};
use lazy_static::lazy_static;
use parking_lot::RwLock;
use smol::spawn;
use std::net::IpAddr;

#[derive(Debug)]
struct Entry {
    host: Arc<str>,
    created: Instant,
}

pub struct DnsCache {
    address_map: RwLock<HashMap<IpAddr, Entry>>,
}

const ENTRY_TIMEOUT: Duration = Duration::from_secs(120);

impl DnsCache {
    pub fn new() -> Arc<DnsCache> {
        let s = Arc::new(Self {
            address_map: Default::default(),
        });

        let r = Arc::downgrade(&s);
        spawn(async move {
            loop {
                Timer::after(Duration::from_secs(60)).await;
                if let Some(c) = r.upgrade() {
                    c.clean_up();
                } else {
                    break;
                }
            }
        });

        s
    }

    pub fn global() -> &'static Self {
        lazy_static! {
            static ref CACHE: Arc<DnsCache> = DnsCache::new();
        }

        CACHE.as_ref()
    }

    fn clean_up(&self) {
        let now = Instant::now();
        let mut guard = self.address_map.write();
        guard.retain(|ip, entry| {
            if now.duration_since(entry.created) > ENTRY_TIMEOUT {
                log::info!("Removing IP = {ip}, Entry = {entry:?}");
                false
            } else {
                true
            }
        })
    }

    pub fn cache(&self, dns_response: &[u8]) -> anyhow::Result<()> {
        let pkt = Packet::parse(dns_response).context("Error parsing DNS packet")?;

        for answer in &pkt.answers {
            let addr = match answer.data {
                RData::A(addr) => IpAddr::V4(addr.0),
                RData::AAAA(addr) => IpAddr::V6(addr.0),
                _ => return Ok(()),
            };

            self.address_map.write().insert(
                addr,
                Entry {
                    host: answer.name.to_string().into(),
                    created: Instant::now(),
                },
            );
        }

        Ok(())
    }

    pub fn get(&self, ip: &IpAddr) -> Option<Arc<str>> {
        self.address_map.read().get(&ip).map(|e| e.host.clone())
    }
}
