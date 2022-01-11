use crate::cursor::Cursor;
use crate::socks5::Status::Completed;
use async_std::net::TcpStream;
use futures::{select, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, FutureExt};
use std::fmt::{Debug, Formatter};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, Ipv6Addr};
use tide::log;

#[derive(Debug)]
enum Status<T: Debug> {
    Incomplete,
    Completed(T),
}

type Auth = u8;

const AUTH_NO_PASSWORD: Auth = 0x0;

#[derive(Debug, Clone)]
struct ClientGreeting {
    v: u8,
    auths: Vec<Auth>,
}

impl Parsable for ClientGreeting {
    fn parse(buf: &[u8]) -> anyhow::Result<Status<(usize, Self)>>
    where
        Self: Sized,
    {
        if buf.len() < 2 {
            return Ok(Status::Incomplete);
        }

        if buf[0] != 5 {
            return Err(anyhow::anyhow!("Invalid socks version: {}", buf[0]));
        }

        let auth_len = buf[1] as usize;
        if buf.len() < 2 + auth_len {
            return Ok(Status::Incomplete);
        }

        Ok(Status::Completed((
            2 + auth_len,
            ClientGreeting {
                v: 0x5,
                auths: (&buf[2..(2 + auth_len)]).to_vec(),
            },
        )))
    }
}

#[derive(Debug)]
enum Address {
    IPv4(Ipv4Addr),
    IPv6(Ipv6Addr),
    DomainName(String),
}

impl Default for Address {
    fn default() -> Self {
        Self::IPv4(Ipv4Addr::UNSPECIFIED)
    }
}

impl ToString for Address {
    fn to_string(&self) -> String {
        match self {
            Address::IPv4(a) => a.to_string(),
            Address::IPv6(a) => a.to_string(),
            Address::DomainName(n) => n.clone(),
        }
    }
}

impl Address {
    async fn write(&self, w: &mut (impl AsyncWrite + Unpin + ?Sized)) -> anyhow::Result<()> {
        match self {
            Address::IPv6(a) => {
                let mut buf = [0u8; 17];
                buf[0] = 0x4;
                (&mut buf[1..17]).copy_from_slice(&a.octets());
                w.write_all(&buf[..]).await?
            }

            Address::IPv4(a) => {
                let mut buf = [0u8; 5];
                buf[0] = 0x1;
                (&mut buf[1..5]).copy_from_slice(&a.octets());
                w.write_all(&buf[..]).await?
            }

            Address::DomainName(n) => {
                let mut v = Vec::with_capacity(1 + n.as_bytes().len());
                v.push(n.len() as u8);
                v.extend_from_slice(n.as_bytes());
                w.write_all(&v).await?;
            }
        }

        Ok(())
    }
}

impl Parsable for Address {
    fn parse(buf: &[u8]) -> anyhow::Result<Status<(usize, Self)>>
    where
        Self: Sized,
    {
        if buf.len() < 1 {
            return Ok(Status::Incomplete);
        }

        let addr_type = buf[0];

        match addr_type {
            0x1 => {
                if buf.len() < 5 {
                    return Ok(Status::Incomplete);
                }
                Ok(Completed((
                    5,
                    Address::IPv4(Ipv4Addr::new(buf[1], buf[2], buf[3], buf[4])),
                )))
            }

            0x4 => {
                if buf.len() < 17 {
                    return Ok(Status::Incomplete);
                }
                let mut addr_buf = [0; 16];
                addr_buf.copy_from_slice(&buf[1..17]);
                Ok(Completed((5, Address::IPv6(Ipv6Addr::from(addr_buf)))))
            }

            0x3 => {
                if buf.len() < 1 {
                    return Ok(Status::Incomplete);
                }

                let name_len = buf[1] as usize;
                if buf.len() < 2 + name_len {
                    return Ok(Status::Incomplete);
                }

                Ok(Completed((
                    2 + name_len,
                    Address::DomainName(String::from_utf8_lossy(&buf[2..2 + name_len]).to_string()),
                )))
            }

            _ => return Err(ConnStatusCode(0x08).into()),
        }
    }
}

#[derive(Debug)]
struct ClientConnRequest {
    cmd: u8,
    address: Address,
    port: u16,
}

const CMD_CONNECT_TCP: u8 = 1;
const CMD_BIND_TCP: u8 = 2;
const CMD_BIND_UDP: u8 = 3;

#[derive(Debug, Copy, Clone)]
struct ConnStatusCode(u8);

impl std::fmt::Display for ConnStatusCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ConnStatusCode {}

impl Parsable for ClientConnRequest {
    fn parse(mut buf: &[u8]) -> anyhow::Result<Status<(usize, Self)>>
    where
        Self: Sized,
    {
        let mut buf_consumed = 0;
        if buf.len() < 3 {
            return Ok(Status::Incomplete);
        }

        if buf[0] != 0x5 {
            return Err(ConnStatusCode(0x07).into());
        }

        if buf[2] != 0x00 {
            return Err(ConnStatusCode(0x07).into());
        }

        let cmd = buf[1];

        buf = &buf[3..];
        buf_consumed += 3;

        let address = match Address::parse(&buf)? {
            Status::Incomplete => return Ok(Status::Incomplete),
            Completed((offset, addr)) => {
                buf = &buf[offset..];
                buf_consumed += offset;
                addr
            }
        };

        if buf.len() < 2 {
            return Ok(Status::Incomplete);
        }
        let port = u16::from_be_bytes([buf[0], buf[1]]);
        buf_consumed += 2;

        Ok(Status::Completed((
            buf_consumed,
            ClientConnRequest { address, port, cmd },
        )))
    }
}

trait Parsable: Debug {
    fn parse(buf: &[u8]) -> anyhow::Result<Status<(usize, Self)>>
    where
        Self: Sized;
}

async fn parse_buf<T: Parsable, const N: usize>(
    cursor: &mut Cursor<N>,
    rx: &mut (impl AsyncRead + Unpin + ?Sized),
) -> anyhow::Result<T> {
    loop {
        let bytes_read = rx.read(cursor.remaining_mut()).await?;
        if bytes_read == 0 {
            return Err(anyhow::anyhow!("EOF"));
        }
        cursor.move_position(bytes_read);
        match T::parse(cursor.used())? {
            Status::Incomplete if cursor.remaining_len() == 0 => {
                return Err(anyhow::anyhow!("Invalid greeting format"));
            }
            Status::Incomplete => continue,
            Status::Completed((offset, r)) => {
                cursor.move_used_to_front(offset);
                return Ok(r);
            }
        }
    }
}

async fn write_response(
    tx: &mut (impl AsyncWrite + Unpin + ?Sized),
    status: ConnStatusCode,
    bound_address: Address,
    bound_port: u16,
) -> anyhow::Result<()> {
    tx.write_all(&[0x5, status.0, 0]).await?;
    bound_address.write(tx).await?;
    tx.write_all(&bound_port.to_be_bytes()).await?;
    Ok(())
}

pub async fn serve_socks5(
    rx: &mut (impl AsyncRead + Unpin + ?Sized),
    tx: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<()> {
    let mut cursor: Cursor<512> = Default::default();

    let greeting: ClientGreeting = parse_buf(&mut cursor, rx).await?;
    log::info!("{:?}", greeting);

    if !greeting.auths.is_empty() {
        let auth_method = if greeting
            .auths
            .iter()
            .position(|a| *a == AUTH_NO_PASSWORD)
            .is_some()
        {
            AUTH_NO_PASSWORD
        } else {
            0xFF
        };
        tx.write_all(&[0x05, auth_method]).await?;
    }

    match parse_buf(&mut cursor, rx).await {
        Ok(ClientConnRequest { cmd, address, port }) if cmd == CMD_CONNECT_TCP => {
            let (upstream_rx, upstream_tx) =
                match TcpStream::connect(format!("{}:{}", address.to_string(), port)).await {
                    Ok(c) => c.split(),
                    Err(e) => {
                        let code = ConnStatusCode(match e.kind() {
                            ErrorKind::ConnectionReset | ErrorKind::ConnectionRefused => 0x5,
                            ErrorKind::TimedOut => 0x3,
                            _ => 0x1,
                        });
                        write_response(tx, code, Default::default(), 0).await?;
                        return Err(e.into());
                    }
                };

            write_response(tx, ConnStatusCode(0), Default::default(), 0).await?;

            if let Err(e) = select! {
                result = async_std::io::copy(upstream_rx, tx).fuse() => result,
                result = async_std::io::copy(rx, upstream_tx).fuse() => result,
            } {
                return Err(e.into());
            }
            Ok(())
        }
        Ok(r) => {
            write_response(tx, ConnStatusCode(0x7), Default::default(), 0).await?;
            Err(anyhow::anyhow!("Unsupported command: {}", r.cmd))
        }
        Err(e) if e.is::<ConnStatusCode>() => {
            write_response(tx, *e.downcast_ref().unwrap(), Default::default(), 0).await?;
            Err(e.into())
        }
        Err(e) => {
            write_response(tx, ConnStatusCode(0x01), Default::default(), 0).await?;
            Err(e.into())
        }
    }
}
