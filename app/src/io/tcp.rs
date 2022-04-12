use futures_lite::{AsyncRead, AsyncWrite};

use crate::socks5::Address;
use std::io::{IoSlice, IoSliceMut};
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

type AsyncTcpStream = smol::net::TcpStream;

static TCP_SOCKET_COUNT: AtomicUsize = AtomicUsize::new(0);

pub struct TcpStream(AsyncTcpStream);

impl TcpStream {
    pub async fn connect_raw(a: impl smol::net::AsyncToSocketAddrs) -> anyhow::Result<Self> {
        Ok(Self::from(AsyncTcpStream::connect(a).await?))
    }

    pub async fn connect(a: &Address<'_>) -> anyhow::Result<Self> {
        match a {
            Address::IP(addr) => Ok(TcpStream::from(AsyncTcpStream::connect(addr).await?)),
            Address::Name { host, port } => Ok(TcpStream::from(
                AsyncTcpStream::connect((host.as_ref(), *port)).await?,
            )),
        }
    }

    pub fn is_v4(&self) -> bool {
        match self.0.local_addr() {
            Ok(v) => v.is_ipv4(),
            _ => true,
        }
    }

    #[cfg(unix)]
    pub fn get_original_dst(&self) -> Option<SocketAddr> {
        use std::{net::SocketAddrV4, os::unix::prelude::AsRawFd};

        use nix::sys::socket::{getsockopt, sockopt::OriginalDst};
        let addr = getsockopt(self.0.as_raw_fd(), OriginalDst).ok()?;

        let addr = SocketAddr::V4(SocketAddrV4::new(
            u32::from_be(addr.sin_addr.s_addr).into(),
            u16::from_be(addr.sin_port),
        ));

        match self.local_addr() {
            Ok(a) if a == addr => None,
            _ => Some(addr),
        }
    }

    #[cfg(not(unix))]
    pub fn get_original_dst(&self) -> Option<SocketAddr> {
        None
    }
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
        let count = TCP_SOCKET_COUNT.fetch_sub(1, Ordering::SeqCst);
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

type AsyncTcpListener = smol::net::TcpListener;

pub struct TcpListener(AsyncTcpListener);

impl TcpListener {
    pub fn from(v: AsyncTcpListener) -> Self {
        Self(v)
    }

    pub fn local_addr(&self) -> smol::io::Result<SocketAddr> {
        self.0.local_addr()
    }

    pub async fn bind(addr: &Address<'_>) -> smol::io::Result<Self> {
        let inner = match addr {
            Address::IP(addr) => AsyncTcpListener::bind(addr).await?,
            Address::Name { host, port } => AsyncTcpListener::bind((host.as_ref(), *port)).await?,
        };
        Ok(Self(inner))
    }

    pub async fn accept(&self) -> smol::io::Result<(TcpStream, SocketAddr)> {
        let (stream, addr) = self.0.accept().await?;
        Ok((TcpStream::from(stream), addr))
    }

    #[cfg(target_os = "unix")]
    pub fn as_raw_fd(&self) -> std::os::unix::prelude::RawFd {
        self.0.as_raw_fd()
    }
}
