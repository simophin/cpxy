use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use parking_lot::RwLock;
use serde::Serialize;

use crate::{config::ClientConfig, counter::Counter, protocol::Stats};

#[derive(Default, Serialize, Debug, Clone)]
pub struct UpstreamStatistics {
    pub tx: Arc<Counter>,
    pub rx: Arc<Counter>,
    pub last_activity: Arc<Counter>,
    pub last_latency: Arc<Counter>,
}

#[derive(Default, Debug)]
pub struct ClientStatistics {
    pub upstreams: RwLock<HashMap<String, UpstreamStatistics>>,
}

impl ClientStatistics {
    pub fn new(c: &ClientConfig) -> Self {
        Self {
            upstreams: RwLock::new(
                c.upstreams
                    .iter()
                    .map(|(n, _)| (n.clone(), Default::default()))
                    .collect(),
            ),
        }
    }

    pub fn get_upstream<R>(
        &self,
        name: &str,
        f: impl FnOnce(&UpstreamStatistics) -> R,
    ) -> Option<R> {
        let stream = self.upstreams.read();
        stream.get(name).map(f)
    }

    pub fn update_upstream(&self, name: &str, latency: Duration) {
        self.get_upstream(name, |stats| {
            stats
                .last_activity
                .set(UNIX_EPOCH.elapsed().unwrap().as_secs() as usize);
            stats.last_latency.set(latency.as_millis() as usize);
        });
    }

    pub fn get_protocol_stats(&self, name: &str) -> Option<Stats> {
        self.get_upstream(name, |s| Stats {
            rx: s.rx.clone(),
            tx: s.tx.clone(),
        })
    }
}

impl Serialize for ClientStatistics {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.upstreams.read().serialize(serializer)
    }
}
