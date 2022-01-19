use crate::parse::ParseError;
use bytes::Buf;
use tokio::io::{AsyncWrite, AsyncWriteExt};

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
pub struct ClientConnRequest {
    pub cmd: Command,
    pub address: Address,
}

impl ClientConnRequest {
    pub fn parse(mut buf: &[u8]) -> Result<Option<(usize, Self)>, ParseError> {
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
        w: &mut (impl AsyncWrite + Unpin + ?Sized),
        code: ConnStatusCode,
        bound_addr: &Address,
    ) -> anyhow::Result<()> {
        let mut buf = Vec::with_capacity(3 + bound_addr.write_len());
        buf.extend_from_slice(&[0x5, code.0, 0x00]);
        bound_addr.write(&mut buf)?;
        w.write_all(&buf).await?;
        Ok(())
    }
}
