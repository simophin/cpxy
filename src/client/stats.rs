use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{config::ClientConfig, counter::Counter, protocol::Stats};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct UpstreamStatistics {
    pub tx: Arc<Counter>,
    pub rx: Arc<Counter>,
    pub last_activity: Arc<Counter>,
    pub last_latency: Arc<Counter>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct ClientStatistics {
    pub upstreams: HashMap<String, UpstreamStatistics>,
}

impl ClientStatistics {
    pub fn new(c: &ClientConfig) -> Self {
        Self {
            upstreams: c
                .upstreams
                .iter()
                .map(|(n, _)| (n.clone(), Default::default()))
                .collect(),
        }
    }

    pub fn update_upstream(&self, name: &str, latency: Duration) {
        if let Some(stats) = self.upstreams.get(name) {
            stats
                .last_activity
                .set(UNIX_EPOCH.elapsed().unwrap().as_secs() as usize);
            stats.last_latency.set(latency.as_millis() as usize);
        }
    }

    pub fn get_protocol_stats(&self, name: &str) -> Option<Stats> {
        self.upstreams.get(name).map(|s| Stats {
            rx: s.rx.clone(),
            tx: s.tx.clone(),
        })
    }
}
