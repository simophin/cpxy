use crate::parse::ParseError;
use bytes::Buf;
use std::io::Cursor;
use tokio::io::{AsyncWrite, AsyncWriteExt};

type Auth = u8;

pub const AUTH_NO_PASSWORD: Auth = 0x0;
pub const AUTH_NOT_ACCEPTED: Auth = 0xFF;

#[derive(Debug, Clone)]
pub struct ClientGreeting {
    pub auths: Vec<Auth>,
}

impl ClientGreeting {
    pub fn parse(buf: &[u8]) -> Result<Option<Self>, ParseError> {
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

        let mut auths = vec![0; len];
        buf.copy_to_slice(&mut auths);
        Ok(Some(Self { auths }))
    }

    pub async fn respond(
        auth: Auth,
        t: &mut (impl AsyncWrite + Unpin + ?Sized),
    ) -> anyhow::Result<()> {
        t.write_all(&[0x5, auth]).await?;
        Ok(())
    }
}
