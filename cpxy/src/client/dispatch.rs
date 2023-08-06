use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::RwLock;
use smallvec::SmallVec;

use crate::protocol::{DynamicProtocol, Protocol, ProtocolReporter, ProxyRequest};

use super::settings::UpstreamSettings;

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
