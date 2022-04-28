use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{AsyncRead, AsyncWrite, Sink, Stream};

use crate::{proxy::protocol::ProxyRequest, socks5::Address};

pub mod direct;
pub mod tcpman;
pub mod udpman;

pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> AsyncStream for T {}

pub type BoxedSink = Pin<Box<dyn Sink<(Bytes, Address<'static>), Error = anyhow::Error> + Send>>;
pub type BoxedStream = Pin<Box<dyn Stream<Item = (Bytes, Address<'static>)> + Send>>;

#[async_trait]
pub trait Protocol {
    fn supports(&self, req: &ProxyRequest<'_>) -> bool;

    async fn new_stream_conn(
        &self,
        req: &ProxyRequest<'_>,
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)>;

    async fn new_dgram_conn(
        &self,
        req: &ProxyRequest<'_>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)>;
}
