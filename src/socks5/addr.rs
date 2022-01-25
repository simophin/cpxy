use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
use std::io::Cursor;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;

use bytes::Buf;
use futures_lite::{AsyncWrite, AsyncWriteExt};
use lazy_static::lazy_static;
use regex::Regex;

use crate::parse::ParseError;

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub enum Address {
    IP(SocketAddr),
    Name { host: String, port: u16 },
}

impl Address {
    pub fn get_port(&self) -> u16 {
        match self {
            Self::IP(addr) => addr.port(),
            Self::Name { port, .. } => *port,
        }
    }
}

impl Default for Address {
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

        lazy_static! {
            static ref RE: Regex = Regex::new(r"^(.+?):(\d+?)$").unwrap();
        }

        RE.captures(s)
            .and_then(|cap| match (cap.get(1), cap.get(2)) {
                (Some(host), Some(port)) => Some(Address::Name {
                    host: host.as_str().to_string(),
                    port: port.as_str().parse().ok()?,
                }),
                _ => None,
            })
            .ok_or_else(|| anyhow!("Invalid path {s} for a sock address"))
    }
}

impl Address {
    pub fn parse(buf: &[u8]) -> Result<Option<(usize, Self)>, ParseError> {
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

    pub async fn write(
        &self,
        buf: &mut (impl AsyncWrite + Unpin + Send + Sync),
    ) -> anyhow::Result<()> {
        match self {
            Address::IP(SocketAddr::V4(addr)) => {
                buf.write_all(&[0x1]).await?;
                buf.write_all(&addr.ip().octets()).await?;
                buf.write_all(addr.port().to_be_bytes().as_slice()).await?;
            }
            Address::IP(SocketAddr::V6(addr)) => {
                buf.write_all(&[0x4]).await?;
                buf.write_all(&addr.ip().octets()).await?;
                buf.write_all(addr.port().to_be_bytes().as_slice()).await?;
            }
            Address::Name { host, port } => {
                let host_len: u8 = host.as_bytes().len().try_into()?;
                buf.write_all(&[0x3, host_len]).await?;
                buf.write_all(host.as_bytes()).await?;
                buf.write_all(port.to_be_bytes().as_slice()).await?;
            }
        }

        Ok(())
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IP(addr) => std::fmt::Display::fmt(addr, f),
            Self::Name { host, port } => f.write_fmt(format_args!("{host}:{port}")),
        }
    }
}
