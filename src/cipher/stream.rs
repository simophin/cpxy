use super::suite::BoxedStreamCipher;
use crate::utils::RWBuffer;
use bytes::BufMut;
use futures_lite::{AsyncRead, AsyncWrite};
use pin_project_lite::pin_project;
use std::cmp::{max, min};
use std::io::Error;
use std::pin::Pin;
use std::task::{Context, Poll};

pin_project! {
    pub struct CipherStream<T> {
        #[pin]
        pub(super) inner: T,
        pub(super) name: String,
        pub(super) rd_cipher: Option<BoxedStreamCipher>,
        pub(super) wr_cipher: Option<BoxedStreamCipher>,
        pub(super) init_read_buf: Option<RWBuffer>,
        pub(super) last_written_size: Option<usize>,
        pub(super) wr_buf: Vec<u8>,
    }
}

impl<T> CipherStream<T> {
    pub fn new(
        name: String,
        inner: T,
        rd_cipher: BoxedStreamCipher,
        wr_cipher: BoxedStreamCipher,
        init_read_buf: Option<RWBuffer>,
    ) -> Self {
        Self {
            inner,
            rd_cipher: Some(rd_cipher),
            wr_cipher: Some(wr_cipher),
            name,
            init_read_buf,
            last_written_size: None,
            wr_buf: Default::default(),
        }
    }
}

impl<T: AsyncRead> AsyncRead for CipherStream<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let p = self.project();
        match p.init_read_buf.as_mut() {
            Some(initial) if initial.remaining_read() > 0 => {
                let len = min(initial.remaining_read(), buf.len());
                buf.put_slice(&initial.read_buf()[..len]);
                log::debug!("{}: Read {len} of initial data", p.name);
                initial.advance_read(len);
                cx.waker().wake_by_ref();
                return Poll::Ready(Ok(len));
            }
            Some(_) => *p.init_read_buf = None,
            _ => {}
        };

        let result = p.inner.poll_read(cx, buf);
        log::debug!(
            "{}: Read: polling for underlying data, cache size: {}, result = {result:?}",
            p.name,
            buf.remaining_mut(),
        );

        if let Poll::Ready(Ok(len)) = &result {
            log::debug!("{}: Read {} bytes", p.name, len);
            if *len > 0 {
                match p.rd_cipher.as_mut() {
                    Some(c) if c.will_modify_data() => {
                        log::debug!("{}: Read and encrypt {len} from underlying stream", p.name);
                        c.apply_keystream(&mut buf[..*len]);
                    }

                    Some(_) => *p.rd_cipher = None,
                    _ => {}
                }
            }
        }

        result
    }
}

impl<T: AsyncWrite> AsyncWrite for CipherStream<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        let mut this = self.project();
        match this.wr_cipher.as_mut() {
            Some(c) if c.will_modify_data() => {
                let desired_write_len =
                    min(buf.len(), max(this.last_written_size.unwrap_or(4096), 512));
                this.wr_buf.clear();
                this.wr_buf.extend_from_slice(&buf[..desired_write_len]);
                c.apply_keystream(this.wr_buf.as_mut());

                let actual_written_len =
                    match this.inner.as_mut().poll_write(cx, this.wr_buf.as_slice()) {
                        Poll::Ready(Ok(v)) => v,
                        Poll::Pending => {
                            c.rewind(this.wr_buf.len());
                            return Poll::Pending;
                        }
                        v => return v,
                    };

                log::debug!(
                    "{}: Cipher write, desired = {desired_write_len}, actual = {actual_written_len}, buf len = {}",
                    this.name,
                    buf.len(),
                );

                if actual_written_len < desired_write_len {
                    let rewind_len = desired_write_len - actual_written_len;
                    log::debug!("{}: Rewinding {rewind_len} bytes", this.name);
                    c.rewind(rewind_len);
                }

                if actual_written_len <= buf.len() {
                    log::debug!("{}: Cipher write, wake for next write", this.name);
                    cx.waker().wake_by_ref();
                }

                *this.last_written_size = Some(actual_written_len);
                return Poll::Ready(Ok(actual_written_len));
            }
            Some(_) => {
                *this.wr_cipher = None;
                *this.wr_buf = Vec::with_capacity(0);
            }
            None => {}
        };

        let result = this.inner.as_mut().poll_write(cx, buf);
        log::debug!(
            "{}: Plain write, desired = {}, result = {result:?}",
            this.name,
            buf.len(),
        );
        result
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().inner.as_mut().poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.project().inner.as_mut().poll_close(cx)
    }
}
