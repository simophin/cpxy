use super::suite::BoxedStreamCipher;
use futures_lite::{AsyncRead, AsyncWrite};
use pin_project_lite::pin_project;
use std::cmp::{max, min};
use std::io::Error;
use std::pin::Pin;
use std::task::{Context, Poll};

pin_project! {
    pub struct CipherStream<R, W> {
        #[pin]
        pub(super) r: R,
        #[pin]
        pub(super) w: W,
        pub(super) name: String,
        pub(super) rd_cipher: Option<BoxedStreamCipher>,
        pub(super) wr_cipher: Option<BoxedStreamCipher>,
        pub(super) last_written_size: Option<usize>,
        pub(super) wr_buf: Vec<u8>,
    }
}

impl<R, W> CipherStream<R, W> {
    pub fn new(
        name: String,
        r: R,
        w: W,
        rd_cipher: BoxedStreamCipher,
        wr_cipher: BoxedStreamCipher,
    ) -> Self {
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

impl<T: AsyncRead, W> AsyncRead for CipherStream<T, W> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
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

impl<R, T: AsyncWrite> AsyncWrite for CipherStream<R, T> {
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

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().w.as_mut().poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.project().w.as_mut().poll_close(cx)
    }
}
