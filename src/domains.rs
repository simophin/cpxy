use crate::socks5::Address;
use adblock::engine::Engine;
use anyhow::anyhow;
use lazy_static::lazy_static;
use std::sync::{Arc, RwLock};

fn new_engine() -> anyhow::Result<Arc<RwLock<Engine>>> {
    let mut engine = Engine::new(true);
    if let Err(_) = engine.deserialize(include_bytes!(concat!(env!("OUT_DIR"), "/gfwlist.abp"))) {
        return Err(anyhow!("Unable to initialise engine"));
    }
    Ok(Arc::new(RwLock::new(engine)))
}

fn get_engine() -> Arc<RwLock<Engine>> {
    lazy_static! {
        static ref engine: Arc<RwLock<Engine>> = new_engine().unwrap();
    }
    engine.clone()
}

pub fn matches_gfw(address: &Address) -> anyhow::Result<bool> {
    engine.check_network_urls()
}
