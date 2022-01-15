use crate::parse::{Parsable, ParseError, ParseResult, Writable};
use crate::socks5::Address;
use bytes::{Buf, BufMut};
use std::borrow::Cow;
use std::fmt::Debug;
use std::io::Cursor;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message<'a> {
    Send(Cow<'a, [u8]>),
    SendTo(Address, Cow<'a, [u8]>),
}

impl<'a> Message<'a> {
    fn parse(mut buf: &[u8]) -> ParseResult<Self> {
        if !buf.has_remaining() {
            return Ok(None);
        }

        match buf.get_u8() {
            0 => {
                if buf.remaining() < 2 {
                    return Ok(None);
                }

                let pkt_len = buf.get_u16() as usize;
                if buf.remaining() < pkt_len {
                    return Ok(None);
                }

                let (pkt, rest) = buf.split_at(pkt_len);
                Ok(Some((rest, Self::Send(Cow::Borrowed(pkt)))))
            }

            1 => {
                let (rest, addr) = match Address::parse(buf)? {
                    Some(a) => a,
                    None => return Ok(None),
                };

                buf = rest;

                if buf.remaining() < 2 {
                    return Ok(None);
                }

                let pkt_len = buf.get_u16() as usize;

                let (pkt, rest) = buf.split_at(pkt_len);
                Ok(Some((rest, Self::SendTo(addr, Cow::Borrowed(pkt)))))
            }

            v => Err(ParseError::unexpected("Message type", v, "0 or 1")),
        }
    }
}

impl<'a> Writable for Message<'a> {
    fn write_len(&self) -> usize {
        todo!()
    }

    fn write(&self, buf: &mut impl BufMut) -> bool {
        match self {
            Message::Send(data) => {
                if buf.remaining_mut() < 1 + 2 + data.len() {
                    return false;
                }

                if data.len() > u16::MAX as usize {
                    return false;
                }

                buf.put_u8(0);
                buf.put_u16(data.len() as u16);
                buf.put_slice(data);
            }
            Message::SendTo(addr, data) => {
                if buf.remaining_mut() < 1 + addr.write_len() + 2 + data.len() {
                    return false;
                }

                buf.put_u8(1);

                if !addr.write(buf) {
                    return false;
                }

                if data.len() > u16::MAX as usize {
                    return false;
                }

                buf.put_u16(data.len() as u16);
                buf.put_slice(data);
            }
        }

        true
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn codec_works() {
        let mut buf: Vec<u8> = Default::default();
        let data = b"hello, world";

        assert!(Message::SendTo(Default::default(), Cow::Borrowed(data)).write(&mut buf));
        assert!(Message::Send(Cow::Borrowed(data)).write(&mut buf));

        assert!(buf.len() > 0);

        {
            let (rest, msg1) = Message::parse(&buf)
                .expect("To parse 1st msg")
                .expect("Message to be there");
            let (rest, msg2) = Message::parse(rest)
                .expect("To parse 2nd msg")
                .expect("Message to be there");
            let msg3 = Message::parse(rest).expect("To parse 3rd msg");

            assert_eq!(
                msg1,
                Message::SendTo(Default::default(), Cow::Borrowed(data))
            );
            assert_eq!(msg2, Message::Send(Cow::Borrowed(data)));
            assert_eq!(msg3, None);
        }
    }
}
