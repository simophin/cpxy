use super::Address;
use anyhow::{bail, Context};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::{borrow::Cow, io::Write};

pub struct UdpPacket<T> {
    buf: T,
    data_offset: usize,
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

pub struct UdpRepr<'a> {
    pub addr: Address<'a>,
    pub payload: Cow<'a, [u8]>,
    pub frag_no: u8,
}

impl<'a> UdpRepr<'a> {
    pub fn header_write_len(&self) -> usize {
        3 + self.addr.write_len()
    }

    pub fn write_len(&self) -> usize {
        self.header_write_len() + self.payload.len()
    }

    pub fn write_to(&self, out: &mut impl Write) -> anyhow::Result<()> {
        out.write_u16::<BigEndian>(0)?;
        out.write_u8(self.frag_no)?;
        self.addr.write_to(out)?;
        out.write_all(self.payload.as_ref())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_works() {
        let payload = Cow::Borrowed(b"hello, world".as_ref());
        let addr: Address = "localhost:9090".try_into().unwrap();

        let mut buf = vec![0u8; 0];
        UdpRepr {
            addr: addr.clone(),
            payload: payload.clone(),
            frag_no: 1,
        }
        .write_to(&mut buf)
        .expect("To write");

        let pkt = UdpPacket::new_checked(buf).expect("To have a packet");
        assert_eq!(pkt.addr(), addr);
        assert_eq!(pkt.frag_no(), 1);
        assert_eq!(pkt.payload(), payload.as_ref());
    }
}
