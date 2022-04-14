use derive_more::Deref;
use tokio::net::{ToSocketAddrs, UdpSocket as TokioUdpSocket};

#[derive(Deref)]
pub struct UdpSocket(TokioUdpSocket);

impl UdpSocket {
    pub async fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        Ok(Self(TokioUdpSocket::bind(addr).await?))
    }
}
