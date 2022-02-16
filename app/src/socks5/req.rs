use crate::parse::ParseError;
use bytes::Buf;
use futures_lite::{AsyncWrite, AsyncWriteExt};

use super::Address;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ConnStatusCode(u8);

impl ConnStatusCode {
    pub const GRANTED: Self = ConnStatusCode(0);
    pub const FAILED: Self = ConnStatusCode(0x1);
    pub const UNSUPPORTED_COMMAND: Self = ConnStatusCode(0x7);
}

impl std::fmt::Display for ConnStatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ConnStatusCode {}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Command(u8);

impl Command {
    pub const CONNECT_TCP: Self = Self(1);
    pub const BIND_UDP: Self = Self(3);
}

#[derive(Debug, Clone)]
pub struct ClientConnRequest<'a> {
    pub cmd: Command,
    pub address: Address<'a>,
}

impl<'a> ClientConnRequest<'a> {
    pub fn parse(mut buf: &'a [u8]) -> Result<Option<(usize, Self)>, ParseError> {
        if buf.remaining() < 3 {
            return Ok(None);
        }

        match buf.get_u8() {
            v if v != 0x5 => return Err(ParseError::unexpected("protocol", v, "0x5")),
            _ => {}
        };

        let cmd = buf.get_u8();
        match buf.get_u8() {
            v if v != 0 => return Err(ParseError::unexpected("rsv", v, "0x00")),
            _ => {}
        };

        let (offset, address) = match Address::parse(buf)? {
            None => return Ok(None),
            Some(addr) => addr,
        };

        Ok(Some((
            3 + offset,
            Self {
                cmd: Command(cmd),
                address,
            },
        )))
    }

    pub async fn respond(
        w: &mut (impl AsyncWrite + Unpin + Send + Sync),
        code: ConnStatusCode,
        bound_addr: &Address<'_>,
    ) -> anyhow::Result<()> {
        w.write_all(&[0x5, code.0, 0x00]).await?;
        bound_addr.write(w).await?;
        Ok(())
    }
}
