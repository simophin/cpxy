use super::Address;
use crate::parse::{Parsable, ParseError, ParseResult};
use bytes::Buf;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ConnStatusCode(u8);

impl ConnStatusCode {
    pub const GRANTED: Self = ConnStatusCode(0);
    pub const FAILED: Self = ConnStatusCode(0x1);
    pub const UNSUPPORTED_ADDRESS_TYPE: Self = ConnStatusCode(0x8);
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
    pub const BIND_TCP: Self = Self(2);
    pub const BIND_UDP: Self = Self(3);
}

#[derive(Debug, Clone)]
pub struct ClientConnRequest<'a> {
    pub cmd: Command,
    pub address: Address<'a>,
}

impl<'a> Parsable<'a> for ClientConnRequest<'a> {
    fn parse(mut buf: &'a [u8]) -> ParseResult<'a, Self> {
        if buf.remaining() < 4 {
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

        let (rest, address) = match Address::parse(buf)? {
            None => return Ok(None),
            Some(addr) => addr,
        };

        Ok(Some((
            rest,
            Self {
                cmd: Command(cmd),
                address,
            },
        )))
    }
}

type Auth = u8;

const AUTH_NO_PASSWORD: Auth = 0x0;

#[derive(Debug, Clone)]
pub struct ClientGreeting<'a> {
    auths: &'a [Auth],
}

impl<'a> Parsable<'a> for ClientGreeting<'a> {
    fn parse(mut buf: &'a [u8]) -> ParseResult<'a, Self> {
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

        let (auths, rest) = buf.split_at(len);
        Ok(Some((rest, Self { auths })))
    }
}
