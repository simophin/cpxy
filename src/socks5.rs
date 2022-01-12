use crate::cursor::Cursor;
use crate::parse::{parse_cursor, Parsable, ParseStatus};
use crate::socks5::ParseStatus::Completed;
use async_std::net::ToSocketAddrs;
use futures::{AsyncRead, AsyncWrite, AsyncWriteExt};
use std::borrow::Cow;
use std::fmt::{Debug, Formatter};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;

type Auth = u8;

const AUTH_NO_PASSWORD: Auth = 0x0;

#[derive(Debug, Clone)]
struct ClientGreeting {
    auths: Vec<Auth>,
}

impl Parsable for ClientGreeting {
    fn parse(buf: &[u8]) -> anyhow::Result<ParseStatus<(usize, Self)>>
    where
        Self: Sized,
    {
        if buf.len() < 2 {
            return Ok(ParseStatus::Incomplete);
        }

        if buf[0] != 5 {
            return Err(anyhow::anyhow!("Invalid socks version: {}", buf[0]));
        }

        let auth_len = buf[1] as usize;
        if buf.len() < 2 + auth_len {
            return Ok(ParseStatus::Incomplete);
        }

        Ok(ParseStatus::Completed((
            2 + auth_len,
            ClientGreeting {
                auths: (&buf[2..(2 + auth_len)]).to_vec(),
            },
        )))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum Address {
    IPv4 {
        ip: [u8; 4],
        port: u16,
    },
    IPv6 {
        ip: [u8; 16],
        port: u16,
    },
    DomainName {
        domain: Cow<'static, str>,
        port: u16,
    },
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Address::IPv4 { ip, port } => {
                f.write_fmt(format_args!("{}:{}", Ipv4Addr::from(*ip), port))
            }
            Address::IPv6 { ip, port } => {
                f.write_fmt(format_args!("{}:{}", Ipv6Addr::from(*ip), port))
            }
            Address::DomainName { domain, port } => {
                f.write_fmt(format_args!("{}:{}", domain, port))
            }
        }
    }
}

impl Default for Address {
    fn default() -> Self {
        Self::IPv4 {
            ip: [0; 4],
            port: 0,
        }
    }
}

impl Address {
    pub async fn to_sock_addrs(&self) -> anyhow::Result<SocketAddr> {
        match self {
            Address::IPv4 { ip, port } => Ok(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(*ip),
                *port,
            ))),
            Address::IPv6 { ip, port } => Ok(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(*ip),
                *port,
                0,
                0,
            ))),
            Address::DomainName { domain, port } => (domain.as_ref(), *port)
                .to_socket_addrs()
                .await?
                .next()
                .ok_or_else(|| anyhow::anyhow!("Unable to solve domain {}", domain)),
        }
    }

    pub async fn write(&self, w: &mut (impl AsyncWrite + Unpin + ?Sized)) -> anyhow::Result<()> {
        let port = match self {
            Address::IPv6 { ip, port } => {
                w.write(&[0x4]).await?;
                w.write_all(ip.as_ref()).await?;
                *port
            }

            Address::IPv4 { ip, port } => {
                w.write(&[0x1]).await?;
                w.write_all(ip.as_ref()).await?;
                *port
            }

            Address::DomainName { domain, port } => {
                let bytes = domain.as_bytes();
                w.write_all(&[0x3, bytes.len() as u8]).await?;
                w.write_all(bytes).await?;
                *port
            }
        };

        w.write_all(port.to_be_bytes().as_ref()).await?;

        Ok(())
    }

    pub fn from(s: &str, port: u16) -> anyhow::Result<Self> {
        match IpAddr::from_str(s) {
            Ok(IpAddr::V4(addr)) => Ok(Self::IPv4 {
                ip: addr.octets(),
                port,
            }),
            Ok(IpAddr::V6(addr)) => Ok(Self::IPv6 {
                ip: addr.octets(),
                port,
            }),
            Err(err) => Err(anyhow::anyhow!("Error converting {} to IP: {}", s, err)),
        }
    }
}

impl Parsable for Address {
    fn parse(buf: &[u8]) -> anyhow::Result<ParseStatus<(usize, Self)>>
    where
        Self: Sized,
    {
        if buf.len() < 1 {
            return Ok(ParseStatus::Incomplete);
        }

        let addr_type = buf[0];

        match addr_type {
            0x1 => {
                if buf.len() < 7 {
                    return Ok(ParseStatus::Incomplete);
                }
                let mut ip = [0u8; 4];
                ip.copy_from_slice(&buf[1..5]);
                let port = u16::from_be_bytes([buf[5], buf[6]]);

                Ok(Completed((7, Address::IPv4 { ip, port })))
            }

            0x4 => {
                if buf.len() < 19 {
                    return Ok(ParseStatus::Incomplete);
                }
                let mut ip = [0; 16];
                ip.copy_from_slice(&buf[1..17]);
                let port = u16::from_be_bytes([buf[17], buf[18]]);
                Ok(Completed((19, Address::IPv6 { ip, port })))
            }

            0x3 => {
                if buf.len() < 1 {
                    return Ok(ParseStatus::Incomplete);
                }

                let name_len = buf[1] as usize;
                if buf.len() < 4 + name_len {
                    return Ok(ParseStatus::Incomplete);
                }

                Ok(Completed((
                    4 + name_len,
                    Address::DomainName {
                        domain: Cow::from(
                            String::from_utf8_lossy(&buf[2..2 + name_len]).to_string(),
                        ),
                        port: u16::from_be_bytes([buf[2 + name_len], buf[3 + name_len]]),
                    },
                )))
            }

            _ => return Err(ConnStatusCode(0x08).into()),
        }
    }
}

#[derive(Debug)]
pub struct ClientConnRequest {
    pub cmd: u8,
    pub address: Address,
}

pub const CMD_CONNECT_TCP: u8 = 1;
pub const CMD_BIND_TCP: u8 = 2;
pub const CMD_BIND_UDP: u8 = 3;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ConnStatusCode(u8);

impl ConnStatusCode {
    pub const GRANTED: Self = ConnStatusCode(0);
    pub const FAILED: Self = ConnStatusCode(0x1);
    pub const UNSUPPORTED_ADDRESS_TYPE: Self = ConnStatusCode(0x8);
    pub const UNSUPPORTED_COMMAND: Self = ConnStatusCode(0x7);
}

impl std::fmt::Display for ConnStatusCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ConnStatusCode {}

impl Parsable for ClientConnRequest {
    fn parse(mut buf: &[u8]) -> anyhow::Result<ParseStatus<(usize, Self)>>
    where
        Self: Sized,
    {
        let mut buf_consumed = 0;
        if buf.len() < 3 {
            return Ok(ParseStatus::Incomplete);
        }

        if buf[0] != 0x5 {
            return Err(ConnStatusCode::UNSUPPORTED_COMMAND.into());
        }

        if buf[2] != 0x00 {
            return Err(ConnStatusCode::UNSUPPORTED_COMMAND.into());
        }

        let cmd = buf[1];

        buf = &buf[3..];
        buf_consumed += 3;

        let address = match Address::parse(&buf)? {
            ParseStatus::Incomplete => return Ok(ParseStatus::Incomplete),
            Completed((offset, addr)) => {
                buf_consumed += offset;
                addr
            }
        };

        Ok(ParseStatus::Completed((
            buf_consumed,
            ClientConnRequest { address, cmd },
        )))
    }
}

pub async fn write_response(
    tx: &mut (impl AsyncWrite + Unpin + ?Sized),
    status: ConnStatusCode,
    bound_address: &Address,
) -> anyhow::Result<()> {
    tx.write_all(&[0x5, status.0, 0]).await?;
    bound_address.write(tx).await?;
    Ok(())
}

pub async fn wait_for_handshake(
    rx: &mut (impl AsyncRead + Unpin + ?Sized),
    tx: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<ClientConnRequest> {
    let mut cursor: Cursor<512> = Default::default();
    let greeting: ClientGreeting = parse_cursor(&mut cursor, rx).await?;
    log::info!("{:?}", greeting);

    if !greeting.auths.is_empty() {
        let (auth_method, quit) = if greeting
            .auths
            .iter()
            .position(|a| *a == AUTH_NO_PASSWORD)
            .is_some()
        {
            (AUTH_NO_PASSWORD, false)
        } else {
            (0xFF, true)
        };
        tx.write_all(&[0x05, auth_method]).await?;
        if quit {
            return Err(anyhow::anyhow!("No supported auth methods"));
        }
    }

    match parse_cursor(&mut cursor, rx).await {
        Ok(r) => Ok(r),
        Err(e) if e.is::<ConnStatusCode>() => {
            write_response(tx, *e.downcast_ref().unwrap(), &Default::default()).await?;
            Err(e.into())
        }
        Err(e) => {
            write_response(tx, ConnStatusCode(0x01), &Default::default()).await?;
            Err(e.into())
        }
    }
}

pub struct UdpPacket<'a> {
    pub frag_no: u8,
    pub addr: Address,
    pub data: &'a [u8],
}

impl<'a> UdpPacket<'a> {
    pub fn parse(mut msg: &'a [u8]) -> anyhow::Result<Self> {
        if msg.len() < 4 || msg[0] != 0 || msg[1] != 0 {
            return Err(anyhow::anyhow!("Invalid UDP header"));
        }

        let frag_no = msg[2];
        msg = &msg[3..];
        let addr = match Address::parse(msg)? {
            ParseStatus::Incomplete => return Err(anyhow::anyhow!("Incomplete address")),
            Completed((offset, r)) => {
                msg = &msg[offset..];
                r
            }
        };

        let data = msg;
        Ok(Self {
            frag_no,
            addr,
            data,
        })
    }

    pub async fn write_packet(
        frag_no: u8,
        addr: &Address,
        data: &[u8],
        tx: &mut (impl AsyncWrite + Unpin + ?Sized),
    ) -> anyhow::Result<()> {
        tx.write_all(&[0x00, 0x00, frag_no]).await?;
        addr.write(tx).await?;
        tx.write_all(data).await?;
        Ok(())
    }
}
