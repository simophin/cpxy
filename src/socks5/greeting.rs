use crate::parse::ParseError;
use bytes::Buf;
use futures::{AsyncWrite, AsyncWriteExt};

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

    pub async fn respond(
        auth: Auth,
        t: &mut (impl AsyncWrite + Unpin + ?Sized),
    ) -> anyhow::Result<()> {
        t.write_all(&[0x5, auth]).await?;
        Ok(())
    }
}
