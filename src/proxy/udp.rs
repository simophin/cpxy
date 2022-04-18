use std::{
    borrow::Cow,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

use crate::buf::Buf as MutBuf;
use crate::rt::{mpsc::bounded, spawn};
use anyhow::{bail, Context};
use bytes::Buf;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream};

use crate::socks5::Address;

// Packet structure:
// |packet_type(u8)|payload_len(u16)|address(variable)|payload|
// Address structure:
// |ipv4(u32)|port(u16)|,
// |ipv6(u128)|port(u16)|,
// |host_len(u16)|host(variable)|port(u16)|
#[derive(Debug)]
pub enum Packet<T> {
    PayloadOnly(T),  // TYPE 1
    WithIpv4Addr(T), // TYPE 2
    WithIpv6Addr(T), // TYPE 3
    WithDomainName {
        address_and_payload: T,
        address_len: usize, // name + port
    }, // TYPE 4
}

impl<T: AsRef<[u8]>> Packet<T> {
    pub fn inner(&self) -> &T {
        match self {
            Packet::PayloadOnly(v) => v,
            Packet::WithIpv4Addr(v) => v,
            Packet::WithIpv6Addr(v) => v,
            Packet::WithDomainName {
                address_and_payload,
                ..
            } => address_and_payload,
        }
    }

    pub fn addr<'a>(&'a self) -> Option<Address<'a>> {
        match self {
            Self::PayloadOnly(_) => None,
            Self::WithIpv4Addr(buf) => {
                let mut b = buf.as_ref();
                let ip = Ipv4Addr::new(b.get_u8(), b.get_u8(), b.get_u8(), b.get_u8());
                let port = b.get_u16();
                Some(Address::IP(SocketAddr::new(IpAddr::V4(ip), port)))
            }
            Self::WithIpv6Addr(buf) => {
                let mut b = buf.as_ref();
                let mut ip = [0u8; 16];
                ip.copy_from_slice(&b[..16]);
                b.advance(ip.len());

                let ip = Ipv6Addr::from(ip);
                let port = b.get_u16();
                Some(Address::IP(SocketAddr::new(IpAddr::V6(ip), port)))
            }
            Self::WithDomainName {
                address_and_payload,
                address_len,
            } => {
                let (name, mut port) = address_and_payload.as_ref().split_at(*address_len - 2);
                let name = std::str::from_utf8(name).ok()?;
                Some(Address::Name {
                    host: Cow::Borrowed(name),
                    port: port.get_u16(),
                })
            }
        }
    }

    pub fn payload(&self) -> &[u8] {
        match self {
            Packet::PayloadOnly(buf) => buf.as_ref(),
            Packet::WithIpv4Addr(buf) => &buf.as_ref()[6..],
            Packet::WithIpv6Addr(buf) => &buf.as_ref()[18..],
            Packet::WithDomainName {
                address_and_payload,
                address_len,
            } => &address_and_payload.as_ref()[*address_len..],
        }
    }
}

pub async fn write_packet_async(
    out: &mut (impl AsyncWrite + Unpin + Send + Sync),
    addr: Option<&Address<'_>>,
    payload: &[u8],
) -> anyhow::Result<usize> {
    let payload_len: u16 = payload
        .len()
        .try_into()
        .context("Converting payload len to u16 int")?;
    let mut written = 0usize;
    match addr {
        None => {
            out.write_all(&[1]).await?;
            out.write_all(&payload_len.to_be_bytes()).await?;
            written += 3;
        }
        Some(Address::IP(SocketAddr::V4(addr))) => {
            out.write_all(&[2]).await?;
            out.write_all(&payload_len.to_be_bytes()).await?;
            out.write_all(addr.ip().octets().as_ref()).await?;
            out.write_all(&addr.port().to_be_bytes()).await?;
            written += 9;
        }
        Some(Address::IP(SocketAddr::V6(addr))) => {
            out.write_all(&[3]).await?;
            out.write_all(&payload_len.to_be_bytes()).await?;
            out.write_all(addr.ip().octets().as_ref()).await?;
            out.write_all(&addr.port().to_be_bytes()).await?;
            written += 21;
        }
        Some(Address::Name { host, port }) => {
            out.write_all(&[4]).await?;
            out.write_all(&payload_len.to_be_bytes()).await?;
            let host = host.as_bytes();
            let domain_name_len: u16 = host
                .len()
                .try_into()
                .context("Converting name len to u16 int")?;
            out.write_all(&domain_name_len.to_be_bytes()).await?;
            out.write_all(host).await?;
            out.write_all(&port.to_be_bytes()).await?;
            written += 3 + domain_name_len as usize + 4
        }
    }

    out.write_all(payload).await?;
    written += payload.len();
    Ok(written)
}

struct PacketPayloader(Packet<MutBuf>);

impl AsRef<[u8]> for PacketPayloader {
    fn as_ref(&self) -> &[u8] {
        self.0.payload()
    }
}

impl Packet<MutBuf> {
    pub async fn read_async(
        input: &mut (impl AsyncRead + Unpin + Send + Sync),
    ) -> anyhow::Result<Self> {
        let mut hdrs = [0u8; 3];
        input.read_exact(&mut hdrs).await.context("Read headers")?;

        let mut hdrs = hdrs.as_ref();
        let payload_type = hdrs.get_u8();
        let payload_len = hdrs.get_u16() as usize;

        match payload_type {
            1 => {
                let mut buf = MutBuf::new_with_len(payload_len, payload_len);
                input
                    .read_exact(&mut buf)
                    .await
                    .context("Reading payload")?;
                Ok(Self::PayloadOnly(buf))
            }

            2 => {
                let mut buf = MutBuf::new_with_len(payload_len + 6, payload_len + 6);
                input
                    .read_exact(&mut buf)
                    .await
                    .context("Reading payload")?;
                Ok(Self::WithIpv4Addr(buf))
            }

            3 => {
                let mut buf = MutBuf::new_with_len(payload_len + 18, payload_len + 18);
                input
                    .read_exact(&mut buf)
                    .await
                    .context("Reading payload")?;
                Ok(Self::WithIpv6Addr(buf))
            }

            4 => {
                let mut host_name_len = [0u8; 2];
                input
                    .read_exact(&mut host_name_len)
                    .await
                    .context("Reading host_name_len")?;

                let host_name_len = u16::from_be_bytes(host_name_len) as usize;
                let read_len = payload_len + host_name_len + 2;
                let mut buf = MutBuf::new_with_len(read_len, read_len);
                input
                    .read_exact(&mut buf)
                    .await
                    .context("Reading payload")?;
                Ok(Self::WithDomainName {
                    address_and_payload: buf,
                    address_len: host_name_len + 2,
                })
            }
            v => bail!("Invalid payload type: {v}"),
        }
    }

    pub fn new_packet_stream(
        mut r: impl AsyncRead + Unpin + Send + Sync + 'static,
        initial_addr: Option<Address<'static>>,
    ) -> impl Stream<Item = (impl AsRef<[u8]> + Send + Sync + 'static, Address<'static>)>
           + Unpin
           + Send
           + Sync
           + 'static {
        let (tx, rx) = bounded::<(PacketPayloader, Address<'static>)>(5);

        spawn(async move {
            let mut last_addr = initial_addr;
            while let Ok(buf) = Self::read_async(&mut r).await {
                let addr = match (buf.addr(), last_addr.as_ref()) {
                    (Some(a), _) => {
                        last_addr.replace(a.into_owned());
                        last_addr.as_ref().unwrap().clone()
                    }
                    (None, Some(a)) => a.clone().into_owned(),
                    _ => {
                        log::debug!("No last address received");
                        continue;
                    }
                };

                if tx.send((PacketPayloader(buf), addr)).await.is_err() {
                    break;
                }
            }
        })
        .detach();
        rx
    }
}

pub struct PacketWriter {
    last_addr: Option<Address<'static>>,
}

impl PacketWriter {
    pub fn new() -> Self {
        Self { last_addr: None }
    }

    pub async fn write(
        &mut self,
        out: &mut (impl AsyncWrite + Unpin + Send + Sync),
        addr: &Address<'_>,
        payload: &[u8],
    ) -> anyhow::Result<usize> {
        let dst = match (addr, self.last_addr.as_ref()) {
            (a, Some(last)) if a == last => None,
            (a, _) => {
                self.last_addr.replace(a.clone().into_owned());
                Some(a)
            }
        };

        write_packet_async(out, dst, payload).await
    }
}

#[cfg(test)]
mod tests {
    use crate::rt::block_on;

    use super::*;

    #[test]
    fn encoding_works() {
        block_on(async move {
            let payload = b"hello, world";

            let addresses = ["www.google.com:3567", "1.2.3.4:80", "[::1]:80"];

            for address in addresses {
                let mut buf = vec![0u8; 0];

                let addr: Address = address.parse().unwrap();
                write_packet_async(&mut buf, Some(&addr), payload)
                    .await
                    .expect("To write to buffer");

                let mut input = buf.as_slice();
                let pkt = Packet::read_async(&mut input).await.unwrap();
                assert_eq!(pkt.addr(), Some(addr));
                assert_eq!(pkt.payload(), payload);
                assert_eq!(input.len(), 0);
            }
        });
    }
}
