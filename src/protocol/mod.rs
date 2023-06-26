use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::counter::Counter;
use crate::socks5::Address;

pub mod direct;
pub mod firetcp;
pub mod http;
pub mod socks5;
// pub mod tcpman;

#[cfg(test)]
mod test;

pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> AsyncStream for T {}

#[derive(Default)]
pub struct Stats {
    pub tx: Arc<Counter>,
    pub rx: Arc<Counter>,
}

#[async_trait]
pub trait Protocol {
    async fn new_stream(
        &self,
        dst: &Address<'_>,
        initial_data: Option<&[u8]>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Box<dyn AsyncStream>>;
}
