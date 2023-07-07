#[cfg(unix)]
use std::os::unix::prelude::AsRawFd;
use std::task::Poll;

use crate::protocol::ProtocolReporter;
use pin_project_lite::pin_project;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use std::sync::Arc;

pin_project! {
    pub struct CounterStream<S> {
        #[pin]
        stream: S,
        reporter: Arc<dyn ProtocolReporter>,
    }
}

impl<S> CounterStream<S> {
    pub fn new(stream: S, reporter: Arc<dyn ProtocolReporter>) -> Self {
        Self { stream, reporter }
    }
}

impl<S: AsyncRead> AsyncRead for CounterStream<S> {
    #[inline]
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.project();
        let old_remaining = buf.remaining();
        let result = this.stream.poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) {
            this.reporter.inc_rx(old_remaining - buf.remaining());
        }
        result
    }
}

impl<S: AsyncWrite> AsyncWrite for CounterStream<S> {
    #[inline]
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        let rc = this.stream.poll_write(cx, buf);
        if let Poll::Ready(Ok(len)) = &rc {
            this.reporter.inc_tx(*len);
        }
        rc
    }

    #[inline]
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().stream.poll_flush(cx)
    }

    #[inline]
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().stream.poll_shutdown(cx)
    }

    #[inline]
    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        let rc = this.stream.poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(len)) = &rc {
            this.reporter.inc_tx(*len);
        }
        rc
    }
}

#[cfg(unix)]
pub trait AsRawFdExt: AsRawFd {
    #[cfg(target_os = "linux")]
    fn set_sock_mark(&self, mark: u32) -> std::io::Result<()> {
        use nix::sys::socket::{setsockopt, sockopt::Mark};

        setsockopt(self.as_raw_fd(), Mark, &mark)?;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn set_sock_mark(&self, _: u32) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
impl<T: AsRawFd> AsRawFdExt for T {}

#[cfg(not(unix))]
pub trait AsRawFdExt {
    fn set_sock_mark(&self, _mark: u32) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(not(unix))]
impl AsRawFdExt for async_net::TcpStream {}

#[cfg(not(unix))]
impl AsRawFdExt for async_net::UdpSocket {}
