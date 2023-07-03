use super::StreamCipher;
use bytes::{Bytes, BytesMut};
use pin_project_lite::pin_project;
use serde::{Deserialize, Serialize};
use std::cmp::min;
use std::fmt::{Debug, Formatter};
use std::io::Error;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Serialize, Deserialize)]
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

impl<C: StreamCipher> Eq for CipherConfig<C> {}

impl<C: StreamCipher> PartialEq for CipherConfig<C> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Full { key: k1, iv: i1 }, Self::Full { key: k2, iv: i2 }) => {
                k1 == k2 && i1 == i2
            }
            (
                Self::FirstNBytes {
                    n: n1,
                    key: k1,
                    iv: i1,
                },
                Self::FirstNBytes {
                    n: n2,
                    key: k2,
                    iv: i2,
                },
            ) => n1 == n2 && k1 == k2 && i1 == i2,
            (Self::None, Self::None) => true,
            _ => false,
        }
    }
}

impl<C: StreamCipher> Clone for CipherConfig<C> {
    fn clone(&self) -> Self {
        match self {
            Self::Full { key, iv } => Self::Full {
                key: key.clone(),
                iv: iv.clone(),
            },
            Self::FirstNBytes { n, key, iv } => Self::FirstNBytes {
                n: n.clone(),
                key: key.clone(),
                iv: iv.clone(),
            },
            Self::None => Self::None,
        }
    }
}

impl<C: StreamCipher> Debug for CipherConfig<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CipherConfig::Full { .. } => f
                .debug_struct("CipherConfig::Full")
                .field("key", &"******")
                .field("iv", &"******")
                .finish(),
            CipherConfig::FirstNBytes { n, .. } => f
                .debug_struct("CipherConfig::FirstNBytes")
                .field("n", n)
                .field("key", &"******")
                .field("iv", &"******")
                .finish(),
            CipherConfig::None => f.debug_struct("CipherConfig::None").finish(),
        }
    }
}

pub enum CipherState<C> {
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

impl<C: StreamCipher> CipherState<C> {
    pub fn apply(&mut self, data: &mut [u8]) {
        match self {
            Self::Full(cipher) => cipher.apply_in_place(data),
            Self::FirstNBytes(cipher, n) => {
                let applied = min(n.get(), data.len());
                cipher.apply_in_place(&mut data[..applied]);
                if let Some(new_n) = NonZeroUsize::new(n.get() - applied) {
                    *n = new_n
                } else {
                    *self = Self::None;
                }
            }
            Self::None => {}
        }
    }

    fn will_apply(&self) -> bool {
        match self {
            Self::None => false,
            _ => true,
        }
    }
}

pin_project! {
    pub struct CipherStream<S, SC, RC> {
        #[pin]
        stream: S,
        pending_plaintext_recv: Option<Bytes>,
        send_cipher_state: CipherState<SC>,
        pending_send: BytesMut,
        recv_cipher_state: CipherState<RC>,
    }
}

impl<S, SC, RC> CipherStream<S, SC, RC>
where
    SC: StreamCipher,
    RC: StreamCipher,
{
    pub fn new(
        stream: S,
        send_cipher_state: CipherState<SC>,
        recv_cipher_state: CipherState<RC>,
        pending_plaintext_recv: Option<Bytes>,
    ) -> Self {
        Self {
            stream,
            pending_plaintext_recv,
            send_cipher_state,
            recv_cipher_state,
            pending_send: Default::default(),
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

        if let Some(pending) = this.pending_plaintext_recv {
            let len = min(buf.remaining(), pending.len());
            buf.put_slice(&pending[..len]);
            if len < pending.len() {
                *this.pending_plaintext_recv = Some(pending.slice(len..));
            } else {
                *this.pending_plaintext_recv = None;
            }
            return Poll::Ready(Ok(()));
        }

        let old_len = buf.filled().len();
        ready!(this.stream.poll_read(cx, buf))?;
        let new_len = buf.filled().len();

        this.recv_cipher_state
            .apply(&mut buf.filled_mut()[old_len..new_len]);

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

        if !this.pending_send.is_empty() {
            let send_len = min(buf.len(), this.pending_send.len());
            let sent = ready!(this.stream.poll_write(cx, &this.pending_send[..send_len]))?;
            let _ = this.pending_send.split_to(sent);
            return Poll::Ready(Ok(sent));
        }

        if this.send_cipher_state.will_apply() {
            this.pending_send.extend_from_slice(buf);
            this.send_cipher_state.apply(&mut this.pending_send);
            let sent = ready!(this.stream.poll_write(cx, this.pending_send.as_ref()))?;
            let _ = this.pending_send.split_to(sent);
            Poll::Ready(Ok(sent))
        } else {
            this.stream.poll_write(cx, buf)
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().stream.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().stream.poll_shutdown(cx)
    }
}
