use std::task::Poll;

use cipher::StreamCipher;
use futures::{ready, AsyncRead};
use pin_project_lite::pin_project;

use anyhow::bail;

use super::cipher_error_to_std;

enum ReadState<IC, EC> {
    Init { bytes_left: usize, cipher: IC },
    Established { cipher: Option<EC> },
}

pin_project! {
    pub struct CipherRead<R, IC, EC> {
        #[pin]
        r: R,
        establish_cipher: Option<EC>,
        state: ReadState<IC, EC>,
    }
}

impl<T, IC, EC> CipherRead<T, IC, EC>
where
    IC: StreamCipher,
    EC: StreamCipher,
{
    pub fn new(r: T, initial_size: usize, initial_cipher: IC) -> Self {
        Self {
            r,
            establish_cipher: None,
            state: ReadState::<_, EC>::Init {
                bytes_left: initial_size,
                cipher: initial_cipher,
            },
        }
    }

    pub fn set_establish_cipher(&mut self, cipher: EC) -> anyhow::Result<()> {
        match &self.state {
            ReadState::Established { .. } => {
                bail!("Connection already established, too late for cipher change")
            }
            ReadState::Init { .. } => {}
        };
        self.establish_cipher.replace(cipher);
        Ok(())
    }
}

impl<R: AsyncRead, IC: StreamCipher, EC: StreamCipher> AsyncRead for CipherRead<R, IC, EC> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        if buf.len() == 0 {
            return Poll::Ready(Ok(0));
        }
        let mut this = self.project();
        match this.state {
            ReadState::Init { bytes_left, cipher } => {
                let buf_to_read = buf.len().min(*bytes_left);
                let buf_read = ready!(this.r.as_mut().poll_read(cx, &mut buf[..buf_to_read]))?;
                cipher
                    .try_apply_keystream(&mut buf[..buf_read])
                    .map_err(cipher_error_to_std)?;
                *bytes_left -= buf_read;
                if *bytes_left == 0 {
                    *this.state = ReadState::Established {
                        cipher: this.establish_cipher.take(),
                    };
                }
                Poll::Ready(Ok(buf_read))
            }
            ReadState::Established {
                cipher: Some(cipher),
            } => {
                let buf_read = ready!(this.r.as_mut().poll_read(cx, buf))?;
                cipher
                    .try_apply_keystream(&mut buf[..buf_read])
                    .map_err(cipher_error_to_std)?;
                Poll::Ready(Ok(buf_read))
            }
            ReadState::Established { .. } => this.r.as_mut().poll_read(cx, buf),
        }
    }
}
