use derive_more::Deref;
use tokio::net::{ToSocketAddrs, UdpSocket as TokioUdpSocket};

use crate::socks5::Address;

#[derive(Deref)]
pub struct UdpSocket(TokioUdpSocket);

impl UdpSocket {
    pub async fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        Ok(Self(TokioUdpSocket::bind(addr).await?))
    }

    pub async fn send_to_addr(&self, buf: &[u8], addr: &Address<'_>) -> std::io::Result<usize> {
        match addr {
            Address::IP(addr) => self.send_to(buf, addr).await,
            Address::Name { host, port } => self.send_to(buf, (host.as_ref(), *port)).await,
        }
    }
}
