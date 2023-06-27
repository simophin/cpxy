use super::StreamCipher;
use crate::buf::RWBuffer;
use bytes::{Bytes, BytesMut};
use pin_project_lite::pin_project;
use serde::{Serialize, Serializer};
use std::cmp::min;
use std::io::Error;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Clone)]
pub enum CipherConfig<C: StreamCipher> {
    Full {
        key: C::Key,
        iv: C::Iv,
    },
    FirstNBytes {
        n: NonZeroUsize,
        key: C::Key,
        iv: C::Iv,
    },
    None,
}

enum CipherState<C> {
    Full(C),
    FirstNBytes(C, NonZeroUsize),
    None,
}

impl<C: StreamCipher> From<CipherConfig<C>> for CipherState<C> {
    fn from(value: CipherConfig<C>) -> Self {
        match value {
            CipherConfig::Full { key, iv } => Self::Full(C::new(&key, &iv)),
            CipherConfig::FirstNBytes { n, key, iv } => Self::FirstNBytes(C::new(&key, &iv), n),
            CipherConfig::None => Self::None,
        }
    }
}

pin_project! {
    pub struct CipherStream<S, SC, RC> {
        #[pin]
        stream: S,
        send_cipher_state: CipherState<SC>,
        pending_send: Option<Bytes>,
        recv_cipher_state: CipherState<RC>,
    }
}

impl<S, SC, RC> CipherStream<S, SC, RC>
where
    SC: StreamCipher,
    RC: StreamCipher,
{
    pub fn new(stream: S, send_config: CipherConfig<SC>, receive_config: CipherConfig<RC>) -> Self {
        Self {
            stream,
            send_cipher_state: send_config.into(),
            recv_cipher_state: receive_config.into(),
            pending_send: None,
        }
    }
}

impl<S, SC, RC> AsyncRead for CipherStream<S, SC, RC>
where
    S: AsyncRead + Unpin,
    SC: StreamCipher,
    RC: StreamCipher,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.project();
        let old_len = buf.filled().len();
        ready!(this.stream.poll_read(cx, buf))?;
        let new_len = buf.filled().len();

        match &mut self.recv_cipher_state {
            CipherState::Full(c) => {
                c.apply_in_place(&mut buf.filled_mut()[old_len..new_len]);
            }
            CipherState::FirstNBytes(c, n) => {
                let apply_len = min(new_len - old_len, n.get());
                c.apply_in_place(&mut buf.filled_mut()[old_len..old_len + apply_len]);
                match NonZeroUsize::new(n.get() - apply_len) {
                    Some(remaining) => *n = remaining,
                    None => self.recv_cipher_state = CipherState::None,
                }
            }
            CipherState::None => {}
        }

        Poll::Ready(Ok(()))
    }
}

impl<S, SC, RC> AsyncWrite for CipherStream<S, SC, RC>
where
    S: AsyncWrite + Unpin,
    SC: StreamCipher,
    RC: StreamCipher,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        let mut this = self.project();

        if let Some(pending) = this.pending_send {
            let send_len = min(buf.len(), pending.len());
            let sent = ready!(this.stream.poll_write(cx, &pending[..send_len]))?;
            if sent == pending.len() {
                *this.pending_send = None;
            } else {
                *this.pending_send = Some(pending.slice(sent..));
                return Poll::Ready(Ok(sent));
            }
        }

        return match this.send_cipher_state {
            CipherState::Full(c) => {
                let mut tmp_buf = BytesMut::from(buf);
                c.apply_in_place(&mut tmp_buf);
                let sent = ready!(this.stream.poll_write(cx, &tmp_buf))?;
                if sent < buf.len() {
                    *this.pending_send = Some(tmp_buf.freeze().slice(sent..));
                }
                Poll::Ready(Ok(sent))
            }

            CipherState::FirstNBytes(c, remaining) => {
                let apply_len = min(buf.len(), remaining.get());
                let mut tmp_buf = BytesMut::from(&buf[..apply_len]);
                c.apply_in_place(&mut tmp_buf);
                let sent = ready!(this.stream.poll_write(cx, &tmp_buf))?;
                match NonZeroUsize::new(remaining.get() - sent) {
                    Some(v) => *remaining = v,
                    None => *this.send_cipher_state = CipherState::None,
                }

                if sent < buf.len() {
                    *this.pending_send = Some(tmp_buf.freeze().slice(sent..));
                }
                return Poll::Ready(Ok(sent));
            }

            CipherState::None => this.stream.poll_write(cx, buf),
        };
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().stream.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().stream.poll_shutdown(cx)
    }
}
