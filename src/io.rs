use crate::socks5::Address;
use futures_lite::{AsyncRead, AsyncWrite};
use smol::net::{AsyncToSocketAddrs, TcpStream as AsyncTcpStream, UdpSocket as AsyncUdpSocket};
use std::io::{IoSlice, IoSliceMut};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

pub struct UdpSocket(AsyncUdpSocket);

static UDP_SOCKET_COUNT: AtomicUsize = AtomicUsize::new(0);
static TCP_SOCKET_COUNT: AtomicUsize = AtomicUsize::new(0);

impl From<AsyncUdpSocket> for UdpSocket {
    fn from(s: AsyncUdpSocket) -> Self {
        UDP_SOCKET_COUNT.fetch_add(1, Ordering::Acquire);
        Self(s)
    }
}

impl UdpSocket {
    // pub fn is_v4(&self) -> bool {
    //     match self.0.local_addr() {
    //         Ok(v) => v.is_ipv4(),
    //         _ => true,
    //     }
    // }

    pub async fn bind(v4: bool) -> smol::io::Result<Self> {
        Ok(Self::from(
            AsyncUdpSocket::bind(if v4 {
                SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
            } else {
                SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
            })
            .await?,
        ))
    }

    pub async fn send_to_addr(&self, buf: &[u8], a: &Address) -> smol::io::Result<usize> {
        match a {
            Address::IP(addr) => self.send_to(buf, addr).await,
            Address::Name { host, port } => self.send_to(buf, (host.as_str(), *port)).await,
        }
    }
}

impl Deref for UdpSocket {
    type Target = AsyncUdpSocket;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for UdpSocket {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        let count = UDP_SOCKET_COUNT.fetch_sub(1, Ordering::Acquire);
        log::debug!("Dropping UDP socket. Remaining: {count}");
    }
}

pub struct TcpStream(AsyncTcpStream);

impl TcpStream {
    pub async fn connect_raw(a: impl AsyncToSocketAddrs) -> smol::io::Result<Self> {
        Ok(Self::from(AsyncTcpStream::connect(a).await?))
    }

    pub async fn connect(a: &Address) -> smol::io::Result<Self> {
        match a {
            Address::IP(addr) => Ok(TcpStream::from(AsyncTcpStream::connect(addr).await?)),
            Address::Name { host, port } => Ok(TcpStream::from(
                AsyncTcpStream::connect((host.as_str(), *port)).await?,
            )),
        }
    }

    // pub fn is_v4(&self) -> bool {
    //     match self.0.local_addr() {
    //         Ok(v) => v.is_ipv4(),
    //         _ => true,
    //     }
    // }
}

impl From<AsyncTcpStream> for TcpStream {
    fn from(s: AsyncTcpStream) -> Self {
        TCP_SOCKET_COUNT.fetch_add(1, Ordering::Acquire);
        Self(s)
    }
}

impl Deref for TcpStream {
    type Target = AsyncTcpStream;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TcpStream {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        let count = TCP_SOCKET_COUNT.fetch_sub(1, Ordering::Acquire);
        log::debug!("Dropping TCP socket. Remaining: {count}");
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }

    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_read_vectored(cx, bufs)
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_close(cx)
    }
}
