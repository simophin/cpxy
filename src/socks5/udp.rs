use super::Address;
use anyhow::{bail, Context};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use bytes::Bytes;
use std::{fmt::Debug, io::Write};

pub struct UdpPacket<T> {
    buf: T,
    data_offset: usize,
}

impl<T: AsRef<[u8]>> Debug for UdpPacket<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("UdpPacket(len={})", self.buf.as_ref().len()))
    }
}

impl<T> UdpPacket<T> {
    pub fn into_inner(self) -> T {
        self.buf
    }

    pub fn inner(&self) -> &T {
        &self.buf
    }
}

impl<T: AsRef<[u8]>> UdpPacket<T> {
    pub fn new_checked(buf: T) -> anyhow::Result<Self> {
        let mut b = buf.as_ref();
        let mut offset = 0;

        if b.read_u16::<BigEndian>().context("Reading RSV")? != 0 {
            bail!("RSV is not 0");
        }

        offset += 2;
        let _ = b.read_u8().context("Reading frag_no")?;
        offset += 1;

        offset += Address::parse(b)
            .context("Parsing address")?
            .context("Parsing address")?
            .0;
        Ok(Self {
            buf,
            data_offset: offset,
        })
    }

    pub fn frag_no(&self) -> u8 {
        self.buf.as_ref()[2]
    }

    pub fn addr<'a>(&'a self) -> Address<'a> {
        Address::parse(&self.buf.as_ref()[3..self.data_offset])
            .unwrap()
            .unwrap()
            .1
    }

    pub fn payload(&self) -> &[u8] {
        &self.buf.as_ref()[self.data_offset..]
    }
}

impl UdpPacket<Bytes> {
    pub fn payload_bytes(&self) -> Bytes {
        self.buf.slice(self.data_offset..)
    }
}

pub struct UdpRepr<'a, T> {
    pub addr: &'a Address<'a>,
    pub payload: T,
    pub frag_no: u8,
}

impl<'a, T: AsRef<[u8]>> UdpRepr<'a, T> {
    pub fn header_write_len(&self) -> usize {
        3 + self.addr.write_len()
    }

    pub fn write_len(&self) -> usize {
        self.header_write_len() + self.payload.as_ref().len()
    }

    pub fn to_packet(&self) -> anyhow::Result<UdpPacket<Bytes>> {
        let out_len = self.write_len();
        let mut out = Vec::with_capacity(out_len);
        out.write_u16::<BigEndian>(0)?;
        out.write_u8(self.frag_no)?;
        self.addr.write_to(&mut out)?;
        out.write_all(self.payload.as_ref())?;
        UdpPacket::new_checked(out.into())
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    #[test]
    fn encoding_works() {
        let payload = Cow::Borrowed(b"hello, world".as_ref());
        let addr: Address = "localhost:9090".try_into().unwrap();

        let pkt = UdpRepr {
            addr: &addr,
            payload: payload.clone(),
            frag_no: 1,
        }
        .to_packet()
        .expect("To write");

        assert_eq!(pkt.addr(), addr);
        assert_eq!(pkt.frag_no(), 1);
        assert_eq!(pkt.payload(), payload.as_ref());
    }
}
