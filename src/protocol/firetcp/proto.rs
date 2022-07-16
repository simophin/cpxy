use anyhow::bail;
use async_trait::async_trait;

use super::super::{AsyncStream, Protocol, Stats, TrafficType};
use crate::socks5::Address;

pub struct FireTcp {
    pub server: Address<'static>,
}

#[async_trait]
impl Protocol for FireTcp {
    fn supports(&self, traffic_type: TrafficType) -> bool {
        traffic_type == TrafficType::Stream
    }

    async fn new_stream(
        &self,
        dst: &Address<'_>,
        initial_data: Option<&[u8]>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Box<dyn AsyncStream>> {
        bail!("Stream unsupported")
    }
}
