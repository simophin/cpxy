use std::{sync::Arc, task::Poll};

use futures::{AsyncRead, AsyncWrite};

use crate::counter::Counter;
use pin_project_lite::pin_project;

#[cfg(unix)]
use std::os::unix::prelude::AsRawFd;

pin_project! {
    pub struct AsyncStreamCounter<S> {
        #[pin]
        stream: S,
        rx: Arc<Counter>,
        tx: Arc<Counter>,
    }
}

impl<S> AsyncStreamCounter<S> {
    pub fn new(stream: S, rx: Arc<Counter>, tx: Arc<Counter>) -> Self {
        Self { stream, rx, tx }
    }
}

impl<S: AsyncRead> AsyncRead for AsyncStreamCounter<S> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        let rc = this.stream.poll_read(cx, buf);
        if let Poll::Ready(Ok(len)) = &rc {
            this.rx.inc(*len);
        }
        rc
    }

    fn poll_read_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        let rc = this.stream.poll_read_vectored(cx, bufs);
        if let Poll::Ready(Ok(len)) = &rc {
            this.rx.inc(*len);
        }
        rc
    }
}

impl<S: AsyncWrite> AsyncWrite for AsyncStreamCounter<S> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        let rc = this.stream.poll_write(cx, buf);
        if let Poll::Ready(Ok(len)) = &rc {
            this.tx.inc(*len);
        }
        rc
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().stream.poll_flush(cx)
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().stream.poll_close(cx)
    }

    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        let rc = this.stream.poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(len)) = &rc {
            this.tx.inc(*len);
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
