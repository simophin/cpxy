use crate::parse::ParseError;
use bytes::Buf;
use std::io::Cursor;
use tokio::io::{AsyncWrite, AsyncWriteExt};

type Auth = u8;

pub const AUTH_NO_PASSWORD: Auth = 0x0;
pub const AUTH_NOT_ACCEPTED: Auth = 0xFF;

#[derive(Debug)]
pub struct ClientGreeting<'a> {
    pub auths: &'a [Auth],
}

impl<'a> ClientGreeting<'a> {
    pub fn parse<'buf>(buf: &'buf [u8]) -> Result<Option<(usize, Self)>, ParseError>
    where
        'buf: 'a,
    {
        let mut buf = Cursor::new(buf);
        if buf.remaining() < 2 {
            return Ok(None);
        }

        match buf.get_u8() {
            v if v != 0x5 => return Err(ParseError::unexpected("protocol", v, "0x5")),
            _ => {}
        };

        let len = buf.get_u8() as usize;
        if buf.remaining() < len {
            return Ok(None);
        }

        let offset = buf.position() as usize + len;
        Ok(Some((
            offset,
            Self {
                auths: &buf.into_inner()[..len],
            },
        )))
    }

    pub async fn respond(
        auth: Auth,
        t: &mut (impl AsyncWrite + Unpin + ?Sized),
    ) -> anyhow::Result<()> {
        t.write_all(&[0x5, auth]).await?;
        Ok(())
    }
}
