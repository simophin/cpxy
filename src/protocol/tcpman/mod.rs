use std::{net::SocketAddr, time::Duration};

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::socks5::Address;

use super::{AsyncDgram, AsyncStream, Protocol};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TcpMan {
    address: Address<'static>,
    ssl: bool,
}

#[async_trait]
impl Protocol for TcpMan {
    fn supports(&self, _: &crate::proxy::protocol::ProxyRequest<'_>) -> bool {
        true
    }

    async fn new_stream_conn(
        &self,
        req: &crate::proxy::protocol::ProxyRequest<'_>,
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)> {
        todo!()
    }

    async fn new_dgram_conn(
        &self,
        req: &crate::proxy::protocol::ProxyRequest<'_>,
    ) -> anyhow::Result<Box<dyn AsyncDgram>> {
        todo!()
    }
}
