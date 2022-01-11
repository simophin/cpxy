use crate::cursor::Cursor;
use crate::socks5::Status::Completed;
use futures::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::fmt::{Debug, Formatter};
use std::io::Error;
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

#[derive(Debug)]
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
        Ok(ClientConnRequest { cmd, address, port }) => {}
        Err(e) if e.is::<ConnStatusCode>() => {
            let code: ConnStatusCode = e.downcast().unwrap();
            tx.write_all()
        }
    }
    log::info!("{:?}", conn_req);

    Ok(())
}
