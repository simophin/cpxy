use crate::parse::ParseError;
use anyhow::bail;
use bytes::Buf;
use smallvec::SmallVec;
use tokio::io::{AsyncBufRead, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

type Auth = u8;

pub const AUTH_NO_PASSWORD: Auth = 0x0;
pub const AUTH_NOT_ACCEPTED: Auth = 0xFF;

#[derive(Debug)]
pub struct ClientGreeting {
    pub auths: SmallVec<[Auth; 4]>,
}

impl ClientGreeting {
    pub async fn parse(stream: &mut (impl AsyncBufRead + Unpin)) -> anyhow::Result<Self> {
        let mut buf = [0u8; 2];
        stream.read_exact(&mut buf).await?;
        if buf[0] != 0x5 {
            bail!("Unknown protocol version: {}", buf[0]);
        }
        let len = buf[1] as usize;
        let mut auths = SmallVec::with_capacity(len);
        auths.resize(len, 0);
        stream.read_exact(auths.as_mut_slice()).await?;
        Ok(Self { auths })
    }

    pub async fn to_async_writer(&self, w: &mut (impl AsyncWrite + Unpin)) -> anyhow::Result<()> {
        w.write_all(&[0x5, self.auths.len().try_into()?]).await?;
        w.write_all(&self.auths).await?;
        Ok(())
    }

    pub async fn respond(
        auth: Auth,
        t: &mut (impl AsyncWrite + Unpin + ?Sized),
    ) -> anyhow::Result<()> {
        t.write_all(&[0x5, auth]).await?;
        Ok(())
    }

    pub async fn read_response(r: &mut (impl AsyncRead + Unpin)) -> anyhow::Result<Auth> {
        let mut bufs = [0u8; 2];
        r.read_exact(&mut bufs).await?;
        if bufs[0] != 0x5 {
            bail!("Unknown protocol version: {}", bufs[0]);
        }
        Ok(bufs[1])
    }
}
