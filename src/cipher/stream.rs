use super::suite::BoxedStreamCipher;
use crate::utils::RWBuffer;
use bytes::BufMut;
use pin_project_lite::pin_project;
use std::cmp::min;
use std::io::Error;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pin_project! {
    pub struct CipherStream<T> {
        #[pin]
        pub(super) inner: T,
        pub(super) name: String,
        pub(super) rd_cipher: BoxedStreamCipher,
        pub(super) wr_cipher: BoxedStreamCipher,
        pub(super) init_read_buf: Option<RWBuffer>,
        pub(super) write_buf: RWBuffer,
    }
}

impl<T> CipherStream<T> {
    pub fn new(
        name: String,
        n: usize,
        inner: T,
        rd_cipher: BoxedStreamCipher,
        wr_cipher: BoxedStreamCipher,
        initial_buf: Option<RWBuffer>,
    ) -> Self {
        Self {
            inner,
            rd_cipher,
            wr_cipher,
            name,
            init_read_buf: initial_buf,
            write_buf: RWBuffer::with_capacity(n),
        }
    }
}

impl<T: AsyncRead> AsyncRead for CipherStream<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let p = self.project();
        match p.init_read_buf.as_mut() {
            Some(initial) if initial.remaining_read() > 0 => {
                let len = min(initial.remaining_read(), buf.remaining());
                buf.put_slice(&initial.read_buf()[..len]);
                log::debug!("{}: Read {len} of initial data", p.name);
                initial.advance_read(len);
                cx.waker().wake_by_ref();
                return Poll::Ready(Ok(()));
            }
            Some(_) => *p.init_read_buf = None,
            _ => {}
        };

        let prev_remaining = buf.remaining();
        log::debug!(
            "{}: Read: polling for underlying data, cache size: {}",
            p.name,
            buf.remaining(),
        );

        match p.inner.poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                log::debug!("{}: Read: data available", p.name);
                if buf.remaining() < prev_remaining {
                    let total_filled_len = buf.filled().len();
                    let new_filled_len = prev_remaining - buf.remaining();
                    log::debug!(
                        "{}: Read and encrypt {new_filled_len} from underlying stream",
                        p.name
                    );
                    p.rd_cipher.apply_keystream(
                        &mut buf.filled_mut()[total_filled_len - new_filled_len..],
                    );
                }
                return Poll::Ready(Ok(()));
            }
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
        }
    }
}

impl<T: AsyncWrite> AsyncWrite for CipherStream<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        if self.write_buf.remaining_write() < buf.len() {
            match self.as_mut().poll_flush(cx) {
                Poll::Ready(v) => {
                    v?;
                }
                Poll::Pending => return Poll::Pending,
            }
        };

        let mut this = self.project();
        log::debug!(
            "{}: Requesting to write {} bytes data to cipher",
            this.name,
            buf.len()
        );
        if this.write_buf.remaining_write() < this.write_buf.total_capacity() / 3 {
            // log::debug!("Compacting write buf");
            this.write_buf.compact();
        }

        let write_len = min(this.write_buf.remaining_write(), buf.len());
        // log::debug!("Admitting {write_len} bytes to cipher stream");
        if write_len > 0 {
            this.write_buf.write_buf().put_slice(&buf[..write_len]);
            // log::debug!(
            //     "Remaining write buffer: {}",
            //     this.write_buf.remaining_write()
            // );
            this.wr_cipher
                .apply_keystream(&mut this.write_buf.write_buf()[..write_len]);
            this.write_buf.advance_write(write_len);
        }
        while this.write_buf.remaining_read() > 0 {
            match this
                .inner
                .as_mut()
                .poll_write(cx, this.write_buf.read_buf())
            {
                Poll::Ready(Ok(n)) => {
                    this.write_buf.advance_read(n);
                    log::debug!(
                        "{}: Wrote {n} bytes cipher stream into underlying. {} remained to be written",
                        this.name,
                        this.write_buf.remaining_read()
                    );
                    if this.write_buf.remaining_read() == 0 {
                        break;
                    }
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => {
                    break;
                }
            }
        }

        if write_len > 0 {
            Poll::Ready(Ok(write_len))
        } else {
            Poll::Pending
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let mut this = self.project();
        log::debug!("{}: Flushing cipher stream", this.name);
        while this.write_buf.remaining_read() > 0 {
            match this
                .inner
                .as_mut()
                .poll_write(cx, this.write_buf.read_buf())
            {
                Poll::Ready(Ok(n)) => {
                    this.write_buf.advance_read(n);
                    log::debug!(
                        "{}: Flushed {n} bytes. Remaining: {}",
                        this.name,
                        this.write_buf.remaining_read()
                    );
                }
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            }
        }

        this.inner.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        match self.as_mut().poll_flush(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        };

        self.project().inner.as_mut().poll_shutdown(cx)
    }
}
