use std::{
    borrow::Cow,
    io::IoSlice,
    net::{IpAddr, SocketAddr},
};

use anyhow::Context;
use bytes::{Buf, Bytes};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{socks5::Address, utils::new_vec_uninitialised};
use enum_primitive_derive::Primitive;
use num_traits::FromPrimitive;

// Packet structure:
// |packet_type(u8)|payload_len(u16)|address(variable)|payload|
// Address structure:
// |ipv4(u32)|port(u16)|,
// |ipv6(u128)|port(u16)|,
// |host_len(u16)|host(variable)|port(u16)|

#[derive(Primitive)]
#[repr(u8)]
enum PacketType {
    PayloadOnly = 1,
    WithV4Addr = 2,
    WithV6Addr = 3,
    WithDomainName = 4,
}

pub struct PacketReader {
    last_received: Option<Address<'static>>,
}

async fn read_vec(
    input: &mut (impl AsyncRead + Unpin + Send + Sync),
    len: usize,
) -> std::io::Result<Vec<u8>> {
    let mut payload = new_vec_uninitialised(len);
    input.read_exact(&mut payload).await?;
    Ok(payload)
}

impl PacketReader {
    pub fn new() -> Self {
        Self {
            last_received: None,
        }
    }

    pub fn new_with_initial_addr(addr: Address<'static>) -> Self {
        Self {
            last_received: Some(addr),
        }
    }

    pub async fn read(
        &mut self,
        input: &mut (impl AsyncRead + Unpin + Send + Sync),
    ) -> anyhow::Result<(Bytes, &Address<'static>)> {
        let mut hdrs = [0u8; 3];
        input
            .read_exact(hdrs.as_mut_slice())
            .await
            .context("Read headers")?;

        let mut hdrs = hdrs.as_ref();
        let packet_type = PacketType::from_u8(hdrs.get_u8()).context("Convert payload type")?;
        let payload_len = hdrs.get_u16() as usize;

        match packet_type {
            PacketType::PayloadOnly => Ok((
                read_vec(input, payload_len)
                    .await
                    .context("Reading payload")?
                    .into(),
                self.last_received
                    .as_ref()
                    .context("Getting last address")?,
            )),
            PacketType::WithV4Addr => {
                let mut data: Bytes = read_vec(input, payload_len + 6)
                    .await
                    .context("Reading payload")?
                    .into();

                let payload = data.split_off(6);
                let mut data = data.as_ref();
                self.last_received =
                    Some(SocketAddr::new(IpAddr::V4(data.get_u32().into()), data.get_u16()).into());
                Ok((payload, self.last_received.as_ref().unwrap()))
            }
            PacketType::WithV6Addr => {
                let mut data: Bytes = read_vec(input, payload_len + 18)
                    .await
                    .context("Reading payload")?
                    .into();

                let payload = data.split_off(18);
                let mut data = data.as_ref();
                self.last_received = Some(
                    SocketAddr::new(IpAddr::V6(data.get_u128().into()), data.get_u16()).into(),
                );
                Ok((payload, self.last_received.as_ref().unwrap()))
            }
            PacketType::WithDomainName => {
                let mut host_name_len = [0u8; 2];
                input
                    .read_exact(&mut host_name_len)
                    .await
                    .context("Read host len")?;

                let host_name_len = u16::from_be_bytes(host_name_len) as usize;
                let mut data: Bytes = read_vec(input, host_name_len + 2 + payload_len)
                    .await
                    .context("Reading payload")?
                    .into();

                let payload = data.split_off(host_name_len + 2);
                let data = data.as_ref();
                let name =
                    std::str::from_utf8(&data[..host_name_len]).context("Converting host name")?;

                let port = (&data[host_name_len..]).get_u16();
                let received_addr = Address::Name {
                    host: Cow::Borrowed(name),
                    port,
                };

                if Some(&received_addr) != self.last_received.as_ref() {
                    self.last_received.replace(received_addr.into_owned());
                }

                Ok((payload, self.last_received.as_ref().unwrap()))
            }
        }
    }
}

pub struct PacketWriter {
    last_sent: Option<Address<'static>>,
}

impl PacketWriter {
    pub fn new() -> Self {
        Self { last_sent: None }
    }

    pub async fn write(
        &mut self,
        out: &mut (impl AsyncWrite + Unpin + Send + Sync),
        addr: &Address<'_>,
        payload: &[u8],
    ) -> anyhow::Result<usize> {
        let payload_len: u16 = payload
            .len()
            .try_into()
            .context("Converting payload len to u16 int")?;

        let addr = match self.last_sent.as_ref() {
            Some(last_addr) if last_addr == addr => None,
            _ => {
                self.last_sent.replace(addr.clone().into_owned());
                Some(addr)
            }
        };

        let mut written = 0usize;

        match addr {
            None => {
                out.write_all_vectored(&mut [
                    IoSlice::new(&[PacketType::PayloadOnly as u8]),
                    IoSlice::new(&payload_len.to_be_bytes()),
                    IoSlice::new(payload),
                ])
                .await?;
                written += 3 + payload.len();
            }
            Some(Address::IP(SocketAddr::V4(addr))) => {
                out.write_all_vectored(&mut [
                    IoSlice::new(&[PacketType::WithV4Addr as u8]),
                    IoSlice::new(&payload_len.to_be_bytes()),
                    IoSlice::new(addr.ip().octets().as_ref()),
                    IoSlice::new(addr.port().to_be_bytes().as_ref()),
                    IoSlice::new(payload),
                ])
                .await?;
                written += 9 + payload.len();
            }
            Some(Address::IP(SocketAddr::V6(addr))) => {
                out.write_all_vectored(&mut [
                    IoSlice::new(&[PacketType::WithV6Addr as u8]),
                    IoSlice::new(&payload_len.to_be_bytes()),
                    IoSlice::new(addr.ip().octets().as_ref()),
                    IoSlice::new(addr.port().to_be_bytes().as_ref()),
                    IoSlice::new(payload),
                ])
                .await?;
                written += 21;
            }
            Some(Address::Name { host, port }) => {
                let host = host.as_bytes();
                let domain_name_len: u16 = host
                    .len()
                    .try_into()
                    .context("Converting name len to u16 int")?;

                out.write_all_vectored(&mut [
                    IoSlice::new(&[PacketType::WithDomainName as u8]),
                    IoSlice::new(&payload_len.to_be_bytes()),
                    IoSlice::new(&domain_name_len.to_be_bytes()),
                    IoSlice::new(host),
                    IoSlice::new(&port.to_be_bytes()),
                    IoSlice::new(payload),
                ])
                .await?;

                written += 3 + domain_name_len as usize + 4 + payload.len();
            }
        }

        Ok(written)
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
                PacketWriter::new()
                    .write(&mut buf, &addr, payload)
                    .await
                    .expect("To write to buffer");

                let mut input = buf.as_slice();
                let mut reader = PacketReader::new();
                let received = reader.read(&mut input).await.expect("To read");
                assert_eq!(received.1, &addr);
                assert_eq!(&received.0, payload.as_ref());
                assert_eq!(input.len(), 0);
            }
        });
    }
}
