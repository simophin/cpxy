use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};

use anyhow::bail;
use bytes::Buf;
use futures::{AsyncWrite, AsyncWriteExt};

use crate::socks5::Address;

pub const SOCKS4_REQUEST_TCP: u8 = 1;

pub struct Request<'a> {
    pub cmd: u8,
    pub addr: Address<'a>,
}

pub fn parse_socks4_request(mut buf: &[u8]) -> anyhow::Result<Option<(usize, Request)>> {
    if buf.remaining() < 9 {
        return Ok(None);
    }

    let mut offset = 0;
    match buf.get_u8() {
        0x4 => {}
        v => bail!("Invalid protocol: {v}"),
    }

    offset += 1;

    let cmd = buf.get_u8();
    offset += 1;

    let port = buf.get_u16();
    offset += 2;

    let ip: [u8; 4] = (&buf[..4]).try_into()?;
    offset += 4;
    buf.advance(4);

    match buf.iter().position(|x| *x == 0) {
        Some(v) => {
            offset += v + 1;
            buf.advance(v + 1);
        }
        None => return Ok(None),
    };

    let addr = if &ip[..3] == [0, 0, 0] && ip[3] != 0 {
        if !buf.has_remaining() {
            return Ok(None);
        }
        match buf.iter().position(|x| *x == 0) {
            Some(v) => {
                offset += v + 1;
                Address::Name {
                    host: String::from_utf8_lossy(&buf[..v]),
                    port,
                }
            }
            None => return Ok(None),
        }
    } else {
        Address::IP(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3])),
            port,
        ))
    };

    Ok(Some((offset, Request { addr, cmd })))
}

pub const SOCKS4_REPLY_GRANTED: u8 = 0x5A;
pub const SOCKS4_REPLY_FAILED: u8 = 0x5B;

pub async fn respond_socks4(
    w: &mut (impl AsyncWrite + Unpin + Send + Sync),
    bound: &SocketAddrV4,
    reply: u8,
) -> anyhow::Result<()> {
    let port = bound.port().to_be_bytes();
    let ip = bound.ip().octets();
    w.write_all(&[0, reply, port[0], port[1], ip[0], ip[1], ip[2], ip[3]])
        .await?;
    Ok(())
}
