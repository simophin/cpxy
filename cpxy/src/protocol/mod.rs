use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

pub mod direct;
pub mod http;
pub mod socks5;
pub mod tcpman;

mod acceptor;
mod dynamic;
mod reporter;
mod req;
pub mod server;
mod stream;
#[cfg(test)]
mod test;

#[async_trait]
pub trait Protocol: Sized {
    type ClientStream: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        reporter: &BoxProtocolReporter,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::ClientStream>;
}

#[async_trait]
impl<P: Protocol + Send + Sync> Protocol for Arc<P> {
    type ClientStream = P::ClientStream;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        reporter: &BoxProtocolReporter,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::ClientStream> {
        self.as_ref().new_stream(req, reporter, fwmark).await
    }
}

pub use acceptor::*;
pub use dynamic::*;
pub use reporter::*;
pub use req::*;
pub use stream::*;
