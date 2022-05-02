use std::io::IoSlice;

use crate::parse::ParseError;
use anyhow::bail;
use bytes::Buf;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

type Auth = u8;

pub const AUTH_NO_PASSWORD: Auth = 0x0;
pub const AUTH_NOT_ACCEPTED: Auth = 0xFF;

#[derive(Debug)]
pub struct ClientGreeting<'a> {
    pub auths: &'a [Auth],
}

impl<'a> ClientGreeting<'a> {
    pub fn parse<'buf>(mut buf: &'buf [u8]) -> Result<Option<(usize, Self)>, ParseError>
    where
        'buf: 'a,
    {
        let mut offset = 0;
        if buf.remaining() < 2 {
            return Ok(None);
        }

        match buf.get_u8() {
            v if v != 0x5 => return Err(ParseError::unexpected("protocol", v, "0x5")),
            _ => offset += 1,
        };

        let len = buf.get_u8() as usize;
        if buf.remaining() < len {
            return Ok(None);
        }

        offset += len + 1;

        Ok(Some((offset, Self { auths: &buf[..len] })))
    }

    pub async fn read_response(r: &mut (impl AsyncRead + Unpin)) -> anyhow::Result<Auth> {
        let mut bufs = [0u8; 2];
        r.read_exact(&mut bufs).await?;
        if bufs[0] != 0x5 {
            bail!("Unknown protocol version: {}", bufs[0]);
        }
        Ok(bufs[1])
    }

    pub async fn to_async_writer(&self, w: &mut (impl AsyncWrite + Unpin)) -> anyhow::Result<()> {
        let hdrs = [0x5u8, self.auths.len().try_into()?];
        let mut out = [IoSlice::new(&hdrs), IoSlice::new(self.auths)];
        w.write_all_vectored(&mut out).await?;
        Ok(())
    }

    pub async fn respond(
        auth: Auth,
        t: &mut (impl AsyncWrite + Unpin + ?Sized),
    ) -> anyhow::Result<()> {
        t.write_all(&[0x5, auth]).await?;
        Ok(())
    }
}
