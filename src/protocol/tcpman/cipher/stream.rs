use pin_project_lite::pin_project;
use std::cmp::{max, min};
use std::io::Error;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::suite::StreamCipherExt;

pin_project! {
    pub struct CipherStream<R, W, RC, WC> {
        #[pin]
        pub(super) r: R,
        #[pin]
        pub(super) w: W,
        pub(super) name: String,
        pub(super) rd_cipher: Option<RC>,
        pub(super) wr_cipher: Option<WC>,
        pub(super) last_written_size: Option<usize>,
        pub(super) wr_buf: Vec<u8>,
    }
}

impl<R, W, RC, WC> CipherStream<R, W, RC, WC> {
    pub fn new(name: String, r: R, w: W, rd_cipher: RC, wr_cipher: WC) -> Self {
        Self {
            r,
            w,
            rd_cipher: Some(rd_cipher),
            wr_cipher: Some(wr_cipher),
            name,
            last_written_size: None,
            wr_buf: Default::default(),
        }
    }
}

impl<T: AsyncRead, W, RC: StreamCipherExt + Send + Sync, WC> AsyncRead
    for CipherStream<T, W, RC, WC>
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let p = self.project();
        let result = p.r.poll_read(cx, buf);
        // log::debug!(
        //     "{}: Read: polling for underlying data, cache size: {}, result = {result:?}",
        //     p.name,
        //     buf.remaining_mut(),
        // );

        if let Poll::Ready(Ok(len)) = &result {
            // log::debug!("{}: Read {} bytes", p.name, len);
            if *len > 0 {
                match p.rd_cipher.as_mut() {
                    Some(c) if c.will_modify_data() => {
                        // log::debug!("{}: Read and encrypt {len} from underlying stream", p.name);
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

impl<R, T: AsyncWrite, RC, WC: StreamCipherExt + Send + Sync> AsyncWrite
    for CipherStream<R, T, RC, WC>
{
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
                    match this.w.as_mut().poll_write(cx, this.wr_buf.as_slice()) {
                        Poll::Ready(Ok(v)) => v,
                        Poll::Pending => {
                            c.rewind(this.wr_buf.len());
                            return Poll::Pending;
                        }
                        v => return v,
                    };

                // log::debug!(
                //     "{}: Cipher write, desired = {desired_write_len}, actual = {actual_written_len}, buf len = {}",
                //     this.name,
                //     buf.len(),
                // );

                if actual_written_len < desired_write_len {
                    let rewind_len = desired_write_len - actual_written_len;
                    // log::debug!("{}: Rewinding {rewind_len} bytes", this.name);
                    c.rewind(rewind_len);
                }

                if actual_written_len <= buf.len() {
                    // log::debug!("{}: Cipher write, wake for next write", this.name);
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

        let result = this.w.as_mut().poll_write(cx, buf);
        // log::debug!(
        //     "{}: Plain write, desired = {}, result = {result:?}",
        //     this.name,
        //     buf.len(),
        // );
        result
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        match self.wr_cipher.as_ref() {
            Some(c) if c.will_modify_data() => {
                let mut total_len = 0usize;
                while let Some(buf) = slice_take_first(&mut bufs) {
                    match self.as_mut().poll_write(cx, buf) {
                        Poll::Ready(Ok(len)) => {
                            total_len += len;
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending if total_len > 0 => return Poll::Ready(Ok(total_len)),
                        Poll::Pending => return Poll::Pending,
                    };

                    if !matches!(self.wr_cipher.as_ref(), Some(c) if c.will_modify_data()) {
                        break;
                    }
                }

                if bufs.is_empty() {
                    Poll::Ready(Ok(total_len))
                } else {
                    match self.project().w.poll_write_vectored(cx, bufs) {
                        Poll::Ready(Ok(v)) => Poll::Ready(Ok(total_len + v)),
                        Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                        Poll::Pending if total_len > 0 => Poll::Ready(Ok(total_len)),
                        Poll::Pending => Poll::Pending,
                    }
                }
            }
            _ => self.project().w.poll_write_vectored(cx, bufs),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().w.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.project().w.as_mut().poll_shutdown(cx)
    }
}

fn slice_take_first<'a, T>(slice: &mut &'a [T]) -> Option<&'a T> {
    if slice.is_empty() {
        return None;
    }

    let (first, rest) = slice.split_at(1);
    *slice = rest;
    Some(&first[0])
}
