use std::io::Result;
use std::net::SocketAddr;

use async_trait::async_trait;

#[async_trait]
pub trait DatagramSocket {
    type RecvType;

    async fn recv_dgram(&self) -> Result<Self::RecvType>;
    async fn send_dgram(&self, buf: &[u8], addr: SocketAddr) -> Result<usize>;
}
