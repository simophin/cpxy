use std::{pin::Pin, sync::Arc};

use anyhow::bail;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{AsyncRead, AsyncWrite, Sink, Stream};

use crate::counter::Counter;
use crate::socks5::Address;

pub mod direct;
pub mod http;
pub mod socks5;
pub mod tcpman;
pub mod udpman;

#[cfg(test)]
mod test;

pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> AsyncStream for T {}

pub type BoxedSink = Pin<Box<dyn Sink<(Bytes, Address<'static>), Error = anyhow::Error> + Send>>;
pub type BoxedStream =
    Pin<Box<dyn Stream<Item = anyhow::Result<(Bytes, Address<'static>)>> + Send>>;

#[derive(Default)]
pub struct Stats {
    pub tx: Arc<Counter>,
    pub rx: Arc<Counter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficType {
    Stream,
    Datagram,
}

#[async_trait]
pub trait Protocol {
    fn supports(&self, traffic_type: TrafficType) -> bool;

    async fn new_stream(
        &self,
        _dst: &Address<'_>,
        _initial_data: Option<&[u8]>,
        _stats: &Stats,
        _fwmark: Option<u32>,
    ) -> anyhow::Result<Box<dyn AsyncStream>> {
        bail!("Stream unsupported")
    }

    async fn new_datagram(
        &self,
        _dst: &Address<'_>,
        _initial_data: Bytes,
        _stats: &Stats,
        _fwmark: Option<u32>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        bail!("Datagram unsupported")
    }
}
