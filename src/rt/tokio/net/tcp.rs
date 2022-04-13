use std::{net::SocketAddr, os::unix::prelude::AsRawFd, pin::Pin, task::Poll};

use futures_lite::{AsyncRead, AsyncWrite};
use tokio::{
    io::ReadBuf,
    net::TcpStream as TokioStream,
    net::{TcpListener as TokioListener, ToSocketAddrs},
};

pub struct TcpStream(TokioStream);

impl TcpStream {
    pub async fn connect(addr: impl tokio::net::ToSocketAddrs) -> std::io::Result<Self> {
        Ok(Self(TokioStream::connect(addr).await?))
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.0.local_addr()
    }

    pub fn set_nodelay(&self, nodelay: bool) -> std::io::Result<()> {
        self.0.set_nodelay(nodelay)
    }
}

impl AsRawFd for TcpStream {
    fn as_raw_fd(&self) -> std::os::unix::prelude::RawFd {
        self.0.as_raw_fd()
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let mut buf = ReadBuf::new(buf);
        match tokio::io::AsyncRead::poll_read(Pin::new(&mut self.0), cx, &mut buf) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(buf.capacity() - buf.remaining())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        tokio::io::AsyncWrite::poll_write(Pin::new(&mut self.0), cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_flush(Pin::new(&mut self.0), cx)
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_shutdown(Pin::new(&mut self.0), cx)
    }
}

pub struct TcpListener(TokioListener);

impl TcpListener {
    pub async fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        Ok(Self(TokioListener::bind(addr).await?))
    }

    pub async fn accept(&self) -> std::io::Result<(TcpStream, SocketAddr)> {
        let (stream, addr) = self.0.accept().await?;
        Ok((TcpStream(stream), addr))
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.0.local_addr()
    }
}
