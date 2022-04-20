use std::{net::SocketAddr, os::unix::prelude::AsRawFd};

use tokio::{
    io::Interest,
    net::{ToSocketAddrs, UdpSocket as TokioUdpSocket},
};

pub struct UdpSocket(TokioUdpSocket);

impl UdpSocket {
    pub async fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        Ok(Self(TokioUdpSocket::bind(addr).await?))
    }

    pub async fn read_with<T>(
        &self,
        f: impl FnOnce(&UdpSocket) -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        let _ = self.0.ready(Interest::READABLE).await?;
        f(&self)
    }

    #[inline]
    pub async fn send_to(&self, buf: &[u8], addr: impl ToSocketAddrs) -> std::io::Result<usize> {
        self.0.send_to(buf, addr).await
    }

    #[inline]
    pub async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        self.0.recv_from(buf).await
    }

    #[inline]
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.0.local_addr()
    }
}

impl From<std::net::UdpSocket> for UdpSocket {
    fn from(socket: std::net::UdpSocket) -> Self {
        Self(TokioUdpSocket::from_std(socket).expect("To convert from std"))
    }
}

impl AsRawFd for UdpSocket {
    fn as_raw_fd(&self) -> std::os::unix::prelude::RawFd {
        self.0.as_raw_fd()
    }
}
