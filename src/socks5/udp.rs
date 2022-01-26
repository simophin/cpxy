use super::Address;
use crate::parse::ParseError;
use bytes::Buf;
use futures_lite::{AsyncWrite, AsyncWriteExt};
use std::borrow::Cow;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UdpPacket<'a> {
    pub frag_no: u8,
    pub addr: Address,
    pub data: Cow<'a, [u8]>,
}

impl<'a> UdpPacket<'a> {
    pub fn parse_tcp<'buf>(mut b: &'buf [u8]) -> Result<Option<(usize, UdpPacket<'a>)>, ParseError>
    where
        'buf: 'a,
    {
        let mut total_offset = 0;
        let addr = match Address::parse(b) {
            Ok(Some((offset, v))) => {
                b.advance(offset);
                total_offset += offset;
                v
            }
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        };

        if b.remaining() < 2 {
            return Ok(None);
        }
        total_offset += 2;

        let data_len = b.get_u16() as usize;
        total_offset += data_len;
        if b.remaining() < data_len {
            return Ok(None);
        }

        let data = Cow::Borrowed(&b[..data_len]);
        Ok(Some((
            total_offset,
            Self {
                frag_no: 0,
                addr,
                data,
            },
        )))
    }

    pub async fn write_tcp(
        w: &mut (impl AsyncWrite + Unpin + Send + Sync),
        addr: &Address,
        data: &[u8],
    ) -> anyhow::Result<()> {
        let data_len: u16 = data.len().try_into()?;

        addr.write(w).await?;
        w.write_all(data_len.to_be_bytes().as_slice()).await?;
        w.write_all(data).await?;
        Ok(())
    }

    pub fn parse_udp<'buf>(mut b: &'buf [u8]) -> Result<UdpPacket<'a>, ParseError>
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

    pub fn udp_header_write_len(addr: &Address) -> usize {
        3 + addr.write_len()
    }

    pub async fn write_udp(
        &self,
        b: &mut (impl AsyncWrite + Unpin + Send + Sync + ?Sized),
    ) -> anyhow::Result<()> {
        b.write_all(&[0, 0, self.frag_no]).await?;
        self.addr.write(b).await?;
        b.write_all(self.data.as_ref()).await?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use bytes::BufMut;
    use smol::block_on;

    #[test]
    fn encoding_works() {
        block_on(async move {
            let mut buf = Vec::new();

            let pkt = UdpPacket {
                frag_no: 0,
                addr: "localhost:123".parse().unwrap(),
                data: Cow::Borrowed(b"hello, world"),
            };

            pkt.write_udp(&mut buf).await.unwrap();

            assert_eq!(pkt, UdpPacket::parse_udp(buf.as_slice()).unwrap());

            buf.clear();
            UdpPacket::write_tcp(&mut buf, &pkt.addr, pkt.data.as_ref())
                .await
                .unwrap();
            buf.put_slice(b"remaining");

            let (offset, parsed) = UdpPacket::parse_tcp(buf.as_slice()).unwrap().unwrap();
            assert_eq!(pkt, parsed);
            drop(parsed);

            assert_eq!(b"remaining", &buf.as_slice()[offset..]);
        });
    }
}
