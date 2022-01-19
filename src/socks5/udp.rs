use super::Address;
use crate::parse::ParseError;
use anyhow::anyhow;
use bytes::{Buf, BufMut};
use std::borrow::Cow;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UdpPacket<'a> {
    pub frag_no: u8,
    pub addr: Address,
    pub data: Cow<'a, [u8]>,
}

impl<'a> UdpPacket<'a> {
    pub fn parse<'buf>(mut b: &'buf [u8]) -> Result<UdpPacket<'a>, ParseError>
    where
        'buf: 'a,
    {
        if b.remaining() < 3 {
            return Err(ParseError::unexpected(
                "UDP packet length",
                b.remaining(),
                ">=3",
            ));
        }

        match b.get_u16() {
            0 => {}
            v => return Err(ParseError::unexpected("UDP RSV", v, "0x0000")),
        };

        let frag_no = b.get_u8();

        let addr = match Address::parse(b)? {
            None => return Err(ParseError::unexpected("UDP address", "", "valid address")),
            Some((offset, v)) => {
                b.advance(offset);
                v
            }
        };

        Ok(Self {
            addr,
            frag_no,
            data: Cow::Borrowed(b),
        })
    }

    pub fn write_len(&self) -> usize {
        return 3 + self.addr.write_len() + self.data.len();
    }

    pub fn write(&self, b: &mut impl BufMut) -> anyhow::Result<()> {
        if b.remaining_mut() < self.write_len() {
            return Err(anyhow!("Not enough buffer to write"));
        }

        b.put_u16(0);
        b.put_u8(self.frag_no);
        self.addr.write(b)?;
        b.put_slice(self.data.as_ref());
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn encoding_works() {
        let mut buf = Vec::new();

        let pkt = UdpPacket {
            frag_no: 1,
            addr: Address::Name("localhost".to_string(), 123),
            data: Cow::Borrowed(b"hello, world"),
        };

        pkt.write(&mut buf).unwrap();

        assert_eq!(pkt, UdpPacket::parse(buf.as_slice()).unwrap());
    }
}
