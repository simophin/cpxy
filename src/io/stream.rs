use futures::{AsyncRead, AsyncWrite};
use pin_project_lite::pin_project;

pin_project! {
    pub struct StreamUnion<R, W> {
        #[pin]
        r: R,
        #[pin]
        w: W,
    }
}

pub fn union<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(r: R, w: W) -> StreamUnion<R, W> {
    StreamUnion { r, w }
}

impl<R: AsyncRead, W> AsyncRead for StreamUnion<R, W> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.project().r.poll_read(cx, buf)
    }

    fn poll_read_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.project().r.poll_read_vectored(cx, bufs)
    }
}

impl<R, W: AsyncWrite> AsyncWrite for StreamUnion<R, W> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.project().w.poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.project().w.poll_write_vectored(cx, bufs)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.project().w.poll_flush(cx)
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.project().w.poll_close(cx)
    }
}
