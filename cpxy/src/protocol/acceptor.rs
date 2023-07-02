use super::ProxyRequest;
use async_trait::async_trait;
use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

#[async_trait]
pub trait ProtocolAcceptor: Sized {
    type AcceptedState: ProtocolAcceptedState;

    async fn accept(
        &self,
        stream: TcpStream,
    ) -> anyhow::Result<(Self::AcceptedState, ProxyRequest)>;
}

pub enum ProtocolReply {
    Error { message: Option<String> },
    Success { initial_data: Option<Bytes> },
}

#[async_trait]
pub trait ProtocolAcceptedState: Sized {
    type ServerStream: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static;

    async fn reply(self, reply: ProtocolReply) -> anyhow::Result<Self::ServerStream>;
}
