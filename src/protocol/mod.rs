use std::{net::SocketAddr, time::Duration};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{AsyncRead, AsyncWrite, Sink, Stream};

use crate::proxy::protocol::ProxyRequest;

pub mod tcpman;

pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> AsyncStream for T {}

pub trait AsyncDgram:
    Stream<Item = (Bytes, SocketAddr)>
    + Sink<(Bytes, SocketAddr), Error = anyhow::Error>
    + Unpin
    + Send
    + Sync
{
}

impl<T> AsyncDgram for T where
    T: Stream<Item = (Bytes, SocketAddr)>
        + Sink<(Bytes, SocketAddr), Error = anyhow::Error>
        + Unpin
        + Send
        + Sync
{
}

#[async_trait]
pub trait Protocol {
    fn supports(&self, req: &ProxyRequest<'_>) -> bool;

    async fn new_stream_conn(
        &self,
        req: &ProxyRequest<'_>,
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)>;

    async fn new_dgram_conn(&self, req: &ProxyRequest<'_>) -> anyhow::Result<Box<dyn AsyncDgram>>;
}
