use super::StreamCipher;
use pin_project_lite::pin_project;
use serde::{Deserializer, Serialize, Serializer};
use std::cmp::min;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{ready, Poll};
use tokio::io::{AsyncRead, ReadBuf};

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

enum CipherState<C: StreamCipher> {
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

pub struct CipherStreamUninit<S>(pub S);

impl<S> CipherStreamUninit<S> {
    pub fn set_send_config<C: StreamCipher>(
        self,
        config: CipherConfig<C>,
    ) -> CipherStreamSendConfigured<S, C> {
        CipherStreamSendConfigured {
            stream: self.0,
            config,
        }
    }

    pub fn set_receive_config<C: StreamCipher>(
        self,
        config: CipherConfig<C>,
    ) -> CipherStreamReceiveConfigured<S, C> {
        CipherStreamReceiveConfigured {
            stream: self.0,
            config,
        }
    }
}

pub struct CipherStreamSendConfigured<S, SC: StreamCipher> {
    stream: S,
    config: CipherConfig<SC>,
}

impl<S, SC: StreamCipher> CipherStreamSendConfigured<S, SC> {
    pub fn set_receive_config<RC: StreamCipher>(
        self,
        config: CipherConfig<RC>,
    ) -> CipherStream<S, SC, RC> {
        CipherStream {
            stream: self.stream,
            send_config: self.config.into(),
            receive_config: config.into(),
        }
    }
}

pub struct CipherStreamReceiveConfigured<S, RC: StreamCipher> {
    stream: S,
    config: CipherConfig<RC>,
}

impl<S, RC: StreamCipher> CipherStreamReceiveConfigured<S, RC> {
    pub fn set_send_config<SC: StreamCipher>(
        self,
        config: CipherConfig<SC>,
    ) -> CipherStream<S, SC, RC> {
        CipherStream {
            stream: self.stream,
            send_config: config.into(),
            receive_config: self.config.into(),
        }
    }
}

pin_project! {
    pub struct CipherStream<S, SC: StreamCipher, RC: StreamCipher> {
        #[pin]
        stream: S,
        send_config: CipherState<SC>,
        receive_config: CipherState<RC>,
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
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.project();
        let old_len = buf.filled().len();
        ready!(this.stream.poll_read(cx, buf))?;
        let new_len = buf.filled().len();

        match &mut self.receive_config {
            CipherState::Full(c) => {
                c.apply(&mut buf.filled_mut()[old_len..new_len]);
            }
            CipherState::FirstNBytes(c, n) => {
                let apply_len = min(new_len - old_len, n.get());
                c.apply(&mut buf.filled_mut()[old_len..old_len + apply_len]);
                match NonZeroUsize::new(n.get() - apply_len) {
                    Some(remaining) => *n = remaining,
                    None => self.receive_config = CipherState::None,
                }
            }
            CipherState::None => {}
        }

        Poll::Ready(Ok(()))
    }
}
