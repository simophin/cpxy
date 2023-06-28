use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::counter::Counter;
use crate::socks5::Address;

pub mod direct;
// pub mod firetcp;
pub mod http;
pub mod socks5;
// pub mod tcpman;

mod dynamic;
mod stream;
#[cfg(test)]
mod test;

#[derive(Default)]
pub struct Stats {
    pub tx: Arc<Counter>,
    pub rx: Arc<Counter>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ProxyRequest {
    pub dst: Address<'static>,
    pub initial_data: Option<Bytes>,
    pub tls: bool,
}

impl ProxyRequest {
    pub fn plain_tcp(dst: Address<'static>) -> Self {
        Self {
            dst,
            initial_data: None,
            tls: false,
        }
    }
}

#[async_trait]
pub trait Protocol {
    type Stream: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::Stream>;
}

pub use dynamic::*;
pub use stream::*;
