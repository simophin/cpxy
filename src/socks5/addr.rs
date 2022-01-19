use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
use std::io::Cursor;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;

use bytes::{Buf, BufMut};

use crate::parse::{Parsable, ParseError, ParseResult, Writable, WriteError};

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Address {
    IP(SocketAddr),
    Name { host: String, port: u16 },
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

        match sscanf::scanf!(s, "{}:{}", String, u16) {
            Some((host, port)) => Ok(Address::Name { host, port }),
            None => return Err(anyhow!("Invalid path {s} for a sock address")),
        }
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
                    Self::Name {
                        host: String::from_utf8(name_buf).map_err(|_| {
                            ParseError::unexpected("domain name", "invalid utf-8", "valid utf-8")
                        })?,
                        port,
                    },
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
            Address::Name { host, .. } => 1 + host.as_bytes().len(),
        }
    }

    fn write(&self, buf: &mut impl BufMut) -> Result<(), WriteError> {
        if buf.remaining_mut() < 1 {
            return Err(WriteError::not_enough_space("addr_type", 1, 0));
        }

        match self {
            Address::IP(SocketAddr::V4(addr)) => {
                if buf.remaining_mut() < 1 + 4 + 2 {
                    return Err(WriteError::not_enough_space(
                        "v4+port",
                        6,
                        buf.remaining_mut(),
                    ));
                }

                buf.put_u8(0x1);
                buf.put_slice(&addr.ip().octets());
                buf.put_u16(addr.port());
            }
            Address::IP(SocketAddr::V6(addr)) => {
                if buf.remaining_mut() < 1 + 16 + 2 {
                    return Err(WriteError::not_enough_space(
                        "v6+port",
                        18,
                        buf.remaining_mut(),
                    ));
                }

                buf.put_u8(0x4);
                buf.put_slice(&addr.ip().octets());
                buf.put_u16(addr.port());
            }
            Address::Name { host, port } => {
                if buf.remaining_mut() < 1 + 1 + host.as_bytes().len() + 2 {
                    return Err(WriteError::not_enough_space(
                        "name+port",
                        1 + host.as_bytes().len() + 2,
                        buf.remaining_mut(),
                    ));
                }

                if host.len() > u8::MAX as usize {
                    return Err(WriteError::ProtocolError {
                        msg: "host name exceeds 255 bytes",
                    });
                }

                buf.put_u8(0x3);
                buf.put_u8(host.as_bytes().len() as u8);
                buf.put_slice(host.as_bytes());
                buf.put_u16(*port);
            }
        }

        Ok(())
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
