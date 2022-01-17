use anyhow::anyhow;
use std::fmt::{Debug, Formatter};
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;

use bytes::{Buf, BufMut};

use crate::parse::{Parsable, ParseError, ParseResult, Writable};

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Address {
    IP(SocketAddr),
    Name(String, u16),
}

impl<'a> Default for Address {
    fn default() -> Self {
        Self::IP(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)))
    }
}

impl FromStr for Address {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match SocketAddr::from_str(s) {
            Ok(v) => return Ok(Address::IP(v)),
            _ => {}
        };

        let mut splits = s.split(":");
        let host = splits.next().unwrap();
        let port = splits
            .next()
            .and_then(|p| u16::from_str(p).ok())
            .ok_or_else(|| anyhow!("Expecting a numeric port"))?;
        Ok(Address::Name(host.to_string(), port))
    }
}

impl Parsable for Address {
    fn parse(buf: &[u8]) -> ParseResult<Self> {
        let mut buf = Cursor::new(buf);
        if !buf.has_remaining() {
            return Ok(None);
        }

        match buf.get_u8() {
            0x1 => {
                if buf.remaining() < 6 {
                    return Ok(None);
                }

                let mut addr = [0u8; 4];
                buf.copy_to_slice(&mut addr);
                let port = buf.get_u16();
                Ok(Some((
                    buf.position() as usize,
                    Self::IP(SocketAddr::V4(SocketAddrV4::new(
                        Ipv4Addr::from(addr),
                        port,
                    ))),
                )))
            }

            0x3 => {
                if buf.remaining() < 1 {
                    return Ok(None);
                }

                let name_len = buf.get_u8() as usize;
                if buf.remaining() < name_len + 2 {
                    return Ok(None);
                }

                let mut name_buf = vec![0; name_len];
                buf.copy_to_slice(name_buf.as_mut_slice());

                let port = buf.get_u16();
                Ok(Some((
                    buf.position() as usize,
                    Self::Name(
                        String::from_utf8(name_buf).map_err(|_| {
                            ParseError::unexpected("domain name", "invalid utf-8", "valid utf-8")
                        })?,
                        port,
                    ),
                )))
            }

            0x4 => {
                if buf.remaining() < 6 {
                    return Ok(None);
                }

                let mut addr = [0u8; 16];
                buf.copy_to_slice(&mut addr);
                let port = buf.get_u16();
                Ok(Some((
                    buf.position() as usize,
                    Self::IP(SocketAddr::V6(SocketAddrV6::new(
                        Ipv6Addr::from(addr),
                        port,
                        0,
                        0,
                    ))),
                )))
            }

            v => Err(ParseError::unexpected("IP address type", v, "1, 3 or 4")),
        }
    }
}

impl Writable for Address {
    fn write_len(&self) -> usize {
        3 + match self {
            Address::IP(SocketAddr::V4(_)) => 4,
            Address::IP(SocketAddr::V6(_)) => 16,
            Address::Name(name, _) => 1 + name.as_bytes().len(),
        }
    }

    fn write(&self, buf: &mut impl BufMut) -> bool {
        if !buf.has_remaining_mut() {
            return false;
        }

        match self {
            Address::IP(SocketAddr::V4(addr)) => {
                if buf.remaining_mut() < 1 + 4 + 2 {
                    return false;
                }

                buf.put_u8(0x1);
                buf.put_slice(&addr.ip().octets());
                buf.put_u16(addr.port());
            }
            Address::IP(SocketAddr::V6(addr)) => {
                if buf.remaining_mut() < 1 + 16 + 2 {
                    return false;
                }

                buf.put_u8(0x4);
                buf.put_slice(&addr.ip().octets());
                buf.put_u16(addr.port());
            }
            Address::Name(host, port) => {
                if buf.remaining_mut() < 1 + 1 + host.as_bytes().len() + 2 {
                    return false;
                }

                if host.len() > u8::MAX as usize {
                    return false;
                }

                buf.put_u8(0x3);
                buf.put_u8(host.as_bytes().len() as u8);
                buf.put_slice(host.as_bytes());
                buf.put_u16(*port);
            }
        }

        true
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IP(addr) => std::fmt::Display::fmt(addr, f),
            Self::Name(name, port) => f.write_fmt(format_args!("{}:{}", name, port)),
        }
    }
}
