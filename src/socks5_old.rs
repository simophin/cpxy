use crate::cursor::Cursor;
use crate::parse::{parse_cursor, Parsable, ParseError, ParseResult, ParseStatus, Writable};
use crate::socks5::ParseStatus::Completed;
use async_std::net::ToSocketAddrs;
use bytes::{Buf, BufMut};
use futures::{AsyncRead, AsyncWrite, AsyncWriteExt};
use std::borrow::Cow;
use std::fmt::{Debug, Formatter};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;

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
