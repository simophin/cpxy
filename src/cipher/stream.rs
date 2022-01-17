use crate::utils::RWBuffer;
use bytes::BufMut;
use chacha20::cipher::StreamCipher;
use chacha20::ChaCha20;
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
        pub(super) cipher: ChaCha20,
        pub(super) init_read_buf: Option<RWBuffer>,
        pub(super) write_buf: RWBuffer,
    }
}

impl<T> CipherStream<T> {
    pub fn new(n: usize, inner: T, cipher: ChaCha20, initial_buf: Option<RWBuffer>) -> Self {
        Self {
            inner,
            cipher,
            init_read_buf: initial_buf,
            write_buf: RWBuffer::with_capacity(n),
        }
    }

    pub fn into_inner(self) -> T {
        self.inner
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
                initial.advance_read(len);
                return Poll::Ready(Ok(()));
            }
            Some(_) => {
                *p.init_read_buf = None;
            }
            _ => {}
        }

        let prev_remaining = buf.remaining();
        match p.inner.poll_read(cx, buf) {
            Poll::Ready(v) => {
                if buf.remaining() < prev_remaining {
                    let total_filled_len = buf.filled().len();
                    let new_filled_len = prev_remaining - buf.remaining();
                    p.cipher.apply_keystream(
                        &mut buf.filled_mut()[total_filled_len - new_filled_len..],
                    );
                }
                return Poll::Ready(v);
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T: AsyncWrite> AsyncWrite for CipherStream<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        let mut this = self.project();
        if this.write_buf.remaining_write() < buf.len() {
            this.write_buf.compact();
        }

        let write_len = min(this.write_buf.remaining_write(), buf.len());
        if write_len > 0 {
            this.write_buf.write_buf().put_slice(&buf[..write_len]);
            this.write_buf.advance_write(write_len);
            this.cipher.apply_keystream(this.write_buf.read_buf_mut());
        }
        while this.write_buf.remaining_read() > 0 {
            match this
                .inner
                .as_mut()
                .poll_write(cx, this.write_buf.read_buf())
            {
                Poll::Ready(Ok(offset)) => {
                    this.write_buf.advance_read(offset);
                    if this.write_buf.remaining_read() == 0 {
                        break;
                    }
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => {
                    cx.waker().wake_by_ref();
                    break;
                }
            }
        }

        return Poll::Ready(Ok(write_len));
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let mut this = self.project();
        while this.write_buf.remaining_read() > 0 {
            match this
                .inner
                .as_mut()
                .poll_write(cx, this.write_buf.read_buf())
            {
                Poll::Ready(Ok(n)) => this.write_buf.advance_read(n),
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            }
        }

        this.inner.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().inner.as_mut().poll_shutdown(cx)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use chacha20::cipher::NewCipher;
    use chacha20::{Key, Nonce};
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_encryption_works() {
        let key = Key::default();
        let nonce = Nonce::default();
        let mut s1 = CipherStream::new(4096, Vec::<u8>::new(), ChaCha20::new(&key, &nonce), None);
        let mut s2 = CipherStream::new(4096, Vec::<u8>::new(), ChaCha20::new(&key, &nonce), None);

        s1.write_all(b"hello, world").await.unwrap();
        s2.write_all(&s1.into_inner()).await.unwrap();
        assert_eq!(b"hello, world".as_ref(), &s2.into_inner());
    }
}
