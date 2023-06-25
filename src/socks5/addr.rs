use anyhow::{bail, Context};
use byteorder::{BigEndian, WriteBytesExt};
use serde::{Deserialize, Serialize};
use smallvec::{smallvec, SmallVec};
use std::borrow::Cow;
use std::fmt::Formatter;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;

use bytes::Buf;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::lookup_host;

use crate::parse::ParseError;

#[derive(Eq, PartialEq, Clone, Hash)]
pub enum Address<'a> {
    IP(SocketAddr),
    Name { host: Cow<'a, str>, port: u16 },
}

impl<'a> Address<'a> {
    pub fn get_port(&self) -> u16 {
        match self {
            Self::IP(addr) => addr.port(),
            Self::Name { port, .. } => *port,
        }
    }

    pub fn get_host(&'a self) -> Cow<'a, str> {
        match self {
            Self::IP(addr) => Cow::Owned(addr.ip().to_string()),
            Self::Name { host, .. } => Cow::Borrowed(host.as_ref()),
        }
    }
}

impl<'a> Default for Address<'a> {
    fn default() -> Self {
        Self::IP(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)))
    }
}

impl<'a> Serialize for Address<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'a, 'de> Deserialize<'de> for Address<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match <String as Deserialize>::deserialize(deserializer)?.parse() {
            Ok(v) => Ok(v),
            Err(e) => Err(serde::de::Error::custom(format!("{e:?}"))),
        }
    }
}

impl<'a> TryFrom<&'a str> for Address<'a> {
    type Error = anyhow::Error;

    fn try_from(s: &'a str) -> Result<Self, Self::Error> {
        match SocketAddr::from_str(s) {
            Ok(v) => return Ok(Address::IP(v)),
            _ => {}
        };

        let (host, port) = match s.rfind(':') {
            Some(v) if v > 0 && v < s.len() - 1 => (&s[..v], &s[(v + 1)..]),
            _ => bail!("Invalid URL: {s}"),
        };

        Ok(Self::Name {
            host: Cow::Borrowed(host),
            port: port.parse().with_context(|| format!("Parsing URL {s}"))?,
        })
    }
}

impl<'a> From<(Cow<'a, str>, u16)> for Address<'a> {
    fn from((host, port): (Cow<'a, str>, u16)) -> Self {
        match host.parse() {
            Ok(ip) => return Address::IP(SocketAddr::new(ip, port)),
            _ => {}
        };

        Address::Name { host, port }
    }
}

impl<'a> FromStr for Address<'a> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Address::try_from(s)?.into_owned())
    }
}

impl From<SocketAddr> for Address<'_> {
    fn from(addr: SocketAddr) -> Self {
        Self::IP(addr)
    }
}

impl<'a> Address<'a> {
    pub async fn resolve(&self) -> std::io::Result<impl Iterator<Item = SocketAddr>> {
        match self {
            Address::IP(addr) => Ok(vec![*addr].into_iter()),
            Address::Name { host, port } => {
                Ok(lookup_host((host.as_ref(), *port)).await?.into_iter())
            }
        }
    }

    pub async fn resolve_first(&self) -> anyhow::Result<SocketAddr> {
        let mut addresses = self.resolve().await?;
        addresses
            .next()
            .ok_or_else(|| anyhow::anyhow!("Unable to resolve {self}"))
    }

    pub async fn parse_async(r: &mut (impl AsyncRead + Unpin)) -> anyhow::Result<Address<'static>> {
        let mut buf: SmallVec<[u8; 6]> = smallvec![0u8; 6];

        r.read_exact(&mut buf[..1])
            .await
            .context("Reading address type")?;

        match buf[0] {
            0x1 => {
                r.read_exact(&mut buf).await.context("Reading IPv4 addr")?;
                let mut buf = buf.as_ref();

                let ip = IpAddr::V4(Ipv4Addr::from(buf.get_u32()));
                let port = buf.get_u16();
                Ok(Address::IP(SocketAddr::new(ip, port)))
            }

            0x4 => {
                buf.resize(18, 0);
                r.read_exact(&mut buf).await.context("Reading IPv6 addr")?;
                let mut buf = buf.as_ref();

                let ip = IpAddr::V6(Ipv6Addr::from(buf.get_u128()));
                let port = buf.get_u16();
                Ok(Address::IP(SocketAddr::new(ip, port)))
            }

            0x3 => {
                r.read_exact(&mut buf[..1])
                    .await
                    .context("Reading address len")?;
                let addr_len = buf[0] as usize;
                let mut addr_buf = vec![0u8; addr_len];
                r.read_exact(&mut addr_buf)
                    .await
                    .context("Reading address")?;
                r.read_exact(&mut buf[..2]).await.context("Reading port")?;
                let port = buf.as_ref().get_u16();
                let addr = String::from_utf8(addr_buf).context("Parsing address string")?;
                Ok(Address::Name {
                    host: Cow::Owned(addr),
                    port,
                })
            }
            b => {
                bail!("Unknown address type {b}");
            }
        }
    }

    pub fn parse(mut buf: &'a [u8]) -> Result<Option<(usize, Self)>, ParseError> {
        if !buf.has_remaining() {
            return Ok(None);
        }

        let mut position = 1;
        match buf.get_u8() {
            0x1 => {
                if buf.remaining() < 6 {
                    return Ok(None);
                }

                let mut addr = [0u8; 4];
                buf.copy_to_slice(&mut addr);
                let port = buf.get_u16();
                position += 6;
                Ok(Some((
                    position,
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
                position += 1;

                let name_len = buf.get_u8() as usize;
                if buf.remaining() < name_len + 2 {
                    return Ok(None);
                }

                position += name_len + 2;
                let host = String::from_utf8_lossy(&buf[..name_len]);
                buf.advance(name_len);
                Ok(Some((position, (host, buf.get_u16()).into())))
            }

            0x4 => {
                if buf.remaining() < 18 {
                    return Ok(None);
                }

                let mut addr = [0u8; 16];
                buf.copy_to_slice(&mut addr);
                let port = buf.get_u16();
                position += addr.len() + 2;
                Ok(Some((
                    position,
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

    pub fn is_unspecified(&'a self) -> bool {
        match self {
            Self::IP(addr) => addr.ip().is_unspecified(),
            Self::Name { host, .. } => {
                host.eq_ignore_ascii_case("0.0.0.0") || host.eq_ignore_ascii_case("::/128")
            }
        }
    }

    pub fn into_owned(self) -> Address<'static> {
        match self {
            Address::IP(addr) => Address::IP(addr.clone()),
            Address::Name { host, port } => Address::Name {
                host: Cow::Owned(host.into_owned()),
                port,
            },
        }
    }

    pub fn write_len(&self) -> usize {
        1 + match self {
            Self::IP(SocketAddr::V4(_)) => 4,
            Self::IP(SocketAddr::V6(_)) => 16,
            Self::Name { host, .. } => 1 + host.as_bytes().len(),
        } + 2
    }

    pub async fn write(&self, buf: &mut (impl AsyncWrite + Unpin + ?Sized)) -> anyhow::Result<()> {
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

    pub fn write_to(&self, buf: &mut impl Write) -> anyhow::Result<()> {
        match self {
            Address::IP(SocketAddr::V4(addr)) => {
                buf.write_u8(0x1)?;
                buf.write_all(&addr.ip().octets())?;
                buf.write_u16::<BigEndian>(addr.port())?;
            }
            Address::IP(SocketAddr::V6(addr)) => {
                buf.write_u8(0x4)?;
                buf.write_all(&addr.ip().octets())?;
                buf.write_u16::<BigEndian>(addr.port())?;
            }
            Address::Name { host, port } => {
                let host_len: u8 = host.as_bytes().len().try_into()?;
                buf.write_u8(0x3)?;
                buf.write_u8(host_len)?;
                buf.write_all(host.as_bytes())?;
                buf.write_u16::<BigEndian>(*port)?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for Address<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IP(addr) => std::fmt::Display::fmt(addr, f),
            Self::Name { host, port } => f.write_fmt(format_args!("{host}:{port}")),
        }
    }
}

impl std::fmt::Debug for Address<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn test_encoding() {
        struct TestCase {
            input: &'static str,
            is_ip: bool,
            expect_host: &'static str,
            expect_port: u16,
            expect_error: bool,
        }

        let test_cases = [
            TestCase {
                input: "127.0.0.1:80",
                expect_host: "127.0.0.1",
                expect_port: 80,
                expect_error: false,
                is_ip: true,
            },
            TestCase {
                input: "domain.com:80",
                expect_host: "domain.com",
                expect_port: 80,
                expect_error: false,
                is_ip: false,
            },
            TestCase {
                input: "[2001:4860:4860::8888]:23",
                expect_host: "2001:4860:4860::8888",
                expect_port: 23,
                expect_error: false,
                is_ip: true,
            },
            TestCase {
                input: "just_domain.com",
                expect_host: "",
                expect_port: 0,
                expect_error: true,
                is_ip: false,
            },
            TestCase {
                input: "127.0.0.1",
                expect_host: "",
                expect_port: 0,
                expect_error: true,
                is_ip: false,
            },
        ];

        for TestCase {
            input,
            expect_error,
            expect_host,
            expect_port,
            is_ip,
        } in test_cases
        {
            match Address::try_from(input) {
                Ok(addr) => {
                    assert!(!expect_error);
                    assert_eq!(expect_host, addr.get_host().as_ref());
                    assert_eq!(expect_port, addr.get_port());
                    match (&addr, is_ip) {
                        (Address::IP(_), false) => panic!("Expect {addr} to not be an IP"),
                        (Address::Name { .. }, true) => panic!("Expect {addr} to be an IP"),
                        _ => {}
                    };

                    // Test encoding/decoding as socks5 protocol
                    let mut buf = Vec::new();
                    addr.write_to(&mut buf).unwrap();
                    buf.extend_from_slice(b"hello, world");

                    let (offset, parsed) = Address::parse(&buf).unwrap().unwrap();
                    assert_eq!(addr, parsed);
                    assert_eq!(b"hello, world", &buf[offset..]);

                    let mut buf = buf.as_slice();
                    let parsed = block_on(Address::parse_async(&mut buf)).unwrap();
                    assert_eq!(addr, parsed);
                }
                Err(e) => {
                    if !expect_error {
                        panic!("Error parsing input: {input}: {e:?}");
                    }
                }
            }
        }
    }
}
