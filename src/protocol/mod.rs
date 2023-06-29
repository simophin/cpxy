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
pub mod tcpman;
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
}

impl<'a> From<Address<'a>> for ProxyRequest {
    fn from(value: Address<'a>) -> Self {
        Self {
            dst: value,
            initial_data: None,
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
