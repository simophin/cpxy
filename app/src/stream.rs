use std::pin::Pin;

use futures_lite::{io::split, AsyncRead, AsyncWrite};

pub struct AsyncReadWrite<'a>(
    Box<dyn AsyncRead + Unpin + Send + Sync + 'a>,
    Box<dyn AsyncWrite + Unpin + Send + Sync + 'a>,
);

impl<'a> AsyncReadWrite<'a> {
    pub fn new(input: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'a) -> Self {
        let (r, w) = split(input);
        Self(Box::new(r), Box::new(w))
    }
}

impl AsyncRead for AsyncReadWrite<'_> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        AsyncRead::poll_read(Pin::new(self.0.as_mut()), cx, buf)
    }

    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        AsyncRead::poll_read_vectored(Pin::new(self.0.as_mut()), cx, bufs)
    }
}

impl AsyncWrite for AsyncReadWrite<'_> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(self.1.as_mut()), cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(self.1.as_mut()), cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        AsyncWrite::poll_close(Pin::new(self.1.as_mut()), cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write_vectored(Pin::new(self.1.as_mut()), cx, bufs)
    }
}
