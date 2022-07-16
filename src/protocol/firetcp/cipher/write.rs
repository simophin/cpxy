use std::task::Poll;

use cipher::{StreamCipher, StreamCipherSeek};
use futures::{ready, AsyncWrite};
use pin_project_lite::pin_project;

use anyhow::bail;

use crate::utils::new_vec_uninitialised;

use super::cipher_error_to_std;

enum WriteState<IC, EC> {
    Init { bytes_left: usize, cipher: IC },
    Established { cipher: Option<EC> },
}

pin_project! {
    pub struct CipherWrite<W, IC, EC> {
        #[pin]
        w: W,
        buf: Box<[u8]>,
        establish_cipher: Option<EC>,
        state: WriteState<IC, EC>,
    }
}

impl<T, IC, EC> CipherWrite<T, IC, EC>
where
    IC: StreamCipher,
    EC: StreamCipher,
{
    pub fn new(r: T, initial_size: usize, initial_cipher: IC) -> Self {
        Self {
            w: r,
            establish_cipher: None,
            buf: new_vec_uninitialised(initial_size.max(512)).into_boxed_slice(),
            state: WriteState::<_, EC>::Init {
                bytes_left: initial_size,
                cipher: initial_cipher,
            },
        }
    }

    pub fn set_establish_cipher(&mut self, cipher: Option<EC>) -> anyhow::Result<()> {
        match &self.state {
            WriteState::Established { .. } => {
                bail!("Connection already established, too late for cipher change")
            }
            WriteState::Init { .. } => {}
        };
        self.establish_cipher = cipher;
        Ok(())
    }
}

impl<W: AsyncWrite, IC: StreamCipher + StreamCipherSeek, EC: StreamCipher + StreamCipherSeek>
    AsyncWrite for CipherWrite<W, IC, EC>
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if buf.len() == 0 {
            return Poll::Ready(Ok(0));
        }

        let this = self.project();
        match this.state {
            WriteState::Init { bytes_left, cipher } => {
                let buf_to_write = buf.len().min(*bytes_left);
                let output = &mut this.buf[..buf_to_write];
                cipher
                    .apply_keystream_b2b(&buf[..buf_to_write], output)
                    .map_err(cipher_error_to_std)?;
                let bytes_written = ready!(this.w.poll_write(cx, output))?;
                if bytes_written < buf_to_write {
                    rewind(cipher, buf_to_write - bytes_written)?;
                }

                *bytes_left -= bytes_written;
                if *bytes_left == 0 {
                    *this.state = WriteState::Established {
                        cipher: this.establish_cipher.take(),
                    };
                }

                Poll::Ready(Ok(bytes_written))
            }
            WriteState::Established {
                cipher: Some(cipher),
            } => {
                let buf_to_write = buf.len().min(this.buf.len());
                let output = &mut this.buf[..buf_to_write];
                cipher
                    .apply_keystream_b2b(&buf[..buf_to_write], output)
                    .map_err(cipher_error_to_std)?;
                let bytes_written = ready!(this.w.poll_write(cx, output))?;
                if bytes_written < buf_to_write {
                    rewind(cipher, buf_to_write - bytes_written)?;
                }

                Poll::Ready(Ok(bytes_written))
            }
            WriteState::Established { .. } => this.w.poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().w.poll_flush(cx)
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().w.poll_close(cx)
    }
}

fn rewind(cipher: &mut impl StreamCipherSeek, cnt: usize) -> std::io::Result<()> {
    let current_pos: usize = cipher
        .try_current_pos()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    cipher
        .try_seek(current_pos - cnt)
        .map_err(cipher_error_to_std)
}
