use std::{
    io::ErrorKind,
    net::SocketAddr,
    os::unix::prelude::AsRawFd,
    pin::Pin,
    task::{Context, Poll},
};

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

    pub fn poll_readable(self: Pin<&Self>, cx: &mut Context) -> Poll<std::io::Result<()>> {
        self.0.poll_recv_ready(cx)
    }

    pub fn poll_writable(self: Pin<&Self>, cx: &mut Context) -> Poll<std::io::Result<()>> {
        self.0.poll_send_ready(cx)
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

    pub fn try_recv_from(&self, buf: &mut [u8]) -> std::io::Result<Option<(usize, SocketAddr)>> {
        match self.0.try_recv_from(buf) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<Option<usize>> {
        match self.0.try_send_to(buf, addr) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
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
