use crate::parse::{Parsable, ParseError, ParseResult, Writable};
use async_std::net::ToSocketAddrs;
use bytes::{Buf, BufMut};
use std::borrow::Cow;
use std::fmt::Formatter;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Address<'a> {
    V4(Cow<'a, [u8; 4]>, u16),
    V6(Cow<'a, [u8; 16]>, u16),
    Name(Cow<'a, str>, u16),
}

impl<'a> Default for Address<'a> {
    fn default() -> Self {
        Address::V4(Cow::Borrowed(&Address::DEFAULT_V4), 0)
    }
}

impl<'a> Parsable<'a> for Address<'a> {
    fn parse(mut buf: &'a [u8]) -> ParseResult<Self> {
        if !buf.has_remaining() {
            return Ok(None);
        }

        match buf.get_u8() {
            0x1 => {
                if buf.remaining() < 6 {
                    return Ok(None);
                }

                let (addr, mut buf) = buf.split_at(4);
                let port = buf.get_u16();
                Ok(Some((
                    buf,
                    Self::V4(Cow::Borrowed(addr.try_into().unwrap()), port),
                )))
            }

            0x3 => {
                if buf.remaining() < 2 {
                    return Ok(None);
                }

                let name_len = buf.get_u16() as usize;
                if buf.remaining() < name_len + 2 {
                    return Ok(None);
                }

                let (s, mut buf) = buf.split_at(name_len);
                let port = buf.get_u16();
                Ok(Some((buf, Self::Name(String::from_utf8_lossy(s), port))))
            }

            0x4 => {
                if buf.remaining() < 18 {
                    return Ok(None);
                }

                let (addr, mut buf) = buf.split_at(16);
                let port = buf.get_u16();
                Ok(Some((
                    buf,
                    Self::V6(Cow::Borrowed(addr.try_into().unwrap()), port),
                )))
            }

            v => Err(ParseError::unexpected("IP address type", v, "1, 3 or 4")),
        }
    }
}

impl<'a> Writable for Address<'a> {
    fn write_len(&self) -> usize {
        3 + match self {
            Address::V4(ip, _) => ip.len(),
            Address::V6(ip, _) => ip.len(),
            Address::Name(name, _) => 2 + name.len(),
        }
    }

    fn write(&self, buf: &mut impl BufMut) -> bool {
        if !buf.has_remaining_mut() {
            return false;
        }

        match self {
            Address::V4(addr, port) => {
                if buf.remaining_mut() < 1 + addr.len() + 2 {
                    return false;
                }

                buf.put_u8(0x1);
                buf.put_slice(addr.as_ref());
                buf.put_u16(*port);
            }
            Address::V6(addr, port) => {
                if buf.remaining_mut() < 1 + addr.len() + 2 {
                    return false;
                }

                buf.put_u8(0x4);
                buf.put_slice(addr.as_ref());
                buf.put_u16(*port);
            }
            Address::Name(host, port) => {
                if buf.remaining_mut() < 1 + 2 + host.as_bytes().len() + 2 {
                    return false;
                }

                if host.len() > u16::MAX as usize {
                    return false;
                }

                buf.put_u8(0x3);
                buf.put_u16(host.as_bytes().len() as u16);
                buf.put_slice(host.as_bytes());
                buf.put_u16(*port);
            }
        }

        true
    }
}

impl<'a> std::fmt::Display for Address<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::V4(ip, port) => {
                f.write_fmt(format_args!("{}:{}", Ipv4Addr::from(*ip.as_ref()), port))
            }
            Self::V6(ip, port) => {
                f.write_fmt(format_args!("{}:{}", Ipv6Addr::from(*ip.as_ref()), port))
            }
            Self::Name(name, port) => f.write_fmt(format_args!("{}:{}", name, port)),
        }
    }
}

impl<'a> Address<'a> {
    const DEFAULT_V4: [u8; 4] = [0; 4];

    pub async fn to_sock_addrs(&self) -> anyhow::Result<SocketAddr> {
        match self {
            Address::V4(ip, port) => Ok(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(*ip.as_ref()),
                *port,
            ))),
            Address::V6(ip, port) => Ok(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(*ip.as_ref()),
                *port,
                0,
                0,
            ))),
            Address::Name(name, port) => (name.as_ref(), *port)
                .to_socket_addrs()
                .await?
                .next()
                .ok_or_else(|| anyhow::anyhow!("Unable to solve domain {}", name)),
        }
    }

    pub fn from(s: &'a str, port: u16) -> anyhow::Result<Self> {
        match IpAddr::from_str(s) {
            Ok(IpAddr::V4(addr)) => Ok(Self::V4(Cow::Owned(addr.octets()), port)),
            Ok(IpAddr::V6(addr)) => Ok(Self::V6(Cow::Owned(addr.octets()), port)),
            Err(_) => Ok(Self::Name(Cow::Borrowed(s), port)),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_works() {
        let mut buf: Vec<u8> = Vec::new();

        assert!(Address::Name(Cow::Borrowed("localhost"), 8081).write(&mut buf));
        assert!(Address::V4(Cow::Owned(Ipv4Addr::BROADCAST.octets()), 8182).write(&mut buf));
        assert!(Address::V6(Cow::Owned(Ipv6Addr::UNSPECIFIED.octets()), 8183).write(&mut buf));

        let (buf, addr) = Address::parse(buf.as_slice()).unwrap().unwrap();
        assert_eq!(Address::Name(Cow::Borrowed("localhost"), 8081), addr);

        let (buf, addr) = Address::parse(buf).unwrap().unwrap();
        assert_eq!(
            Address::V4(Cow::Owned(Ipv4Addr::BROADCAST.octets()), 8182),
            addr
        );

        let (buf, addr) = Address::parse(buf).unwrap().unwrap();
        assert_eq!(
            Address::V6(Cow::Owned(Ipv6Addr::UNSPECIFIED.octets()), 8183),
            addr
        );
    }
}
