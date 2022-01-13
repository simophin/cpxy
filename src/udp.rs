use bytes::{Buf, BufMut};
use std::borrow::Cow;
use std::fmt::{Debug, Display, Formatter};

#[derive(Debug)]
pub enum ParseError {
    Unexpected {
        name: &'static str,
        expect: &'static str,
        got: Box<dyn Debug>,
    },
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        <ParseError as Debug>::fmt(self, f)
    }
}

impl ParseError {
    pub fn unexpected(name: &'static str, got: impl Debug + 'static, expect: &'static str) -> Self {
        Self::Unexpected {
            name,
            got: Box::new(got),
            expect,
        }
    }
}

impl std::error::Error for ParseError {}

pub type ParseResult<T> = Result<Option<T>, ParseError>;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Address<'a> {
    V4(Cow<'a, [u8; 4]>, u16),
    V6(Cow<'a, [u8; 16]>, u16),
    Name(Cow<'a, str>, u16),
}

impl<'a> Default for Address<'a> {
    fn default() -> Self {
        Address::V4(Cow::Borrowed(&Address::DEFAULT_V4), 0)
    }
}

impl<'a> Address<'a> {
    const DEFAULT_V4: [u8; 4] = [0; 4];

    pub fn parse<'b>(buf: &'b mut &'a [u8]) -> ParseResult<Self> {
        if !buf.has_remaining() {
            return Ok(None);
        }

        match buf.get_u8() {
            0x1 => {
                if buf.remaining() < 6 {
                    return Ok(None);
                }

                let (addr, rest) = buf.split_at(4);
                *buf = rest;
                Ok(Some(Self::V4(
                    Cow::Borrowed(addr.try_into().unwrap()),
                    buf.get_u16(),
                )))
            }

            0x3 => {
                if buf.remaining() < 1 {
                    return Ok(None);
                }

                let name_len = buf.get_u8() as usize;
                if buf.remaining() < name_len + 2 {
                    return Ok(None);
                }

                let (s, rest) = buf.split_at(name_len);
                *buf = rest;
                Ok(Some(Self::Name(String::from_utf8_lossy(s), buf.get_u16())))
            }

            0x4 => {
                if buf.remaining() < 18 {
                    return Ok(None);
                }

                let (addr, rest) = buf.split_at(16);
                *buf = rest;
                Ok(Some(Self::V6(
                    Cow::Borrowed(addr.try_into().unwrap()),
                    buf.get_u16(),
                )))
            }

            v => Err(ParseError::unexpected("IP address type", v, "1, 3 or 4")),
        }
    }

    pub fn write(&self, buf: &mut impl BufMut) -> bool {
        if !buf.has_remaining_mut() {
            return false;
        }

        match self {
            Address::V4(addr, port) => {
                if buf.remaining_mut() < 1 + addr.len() + 2 {
                    return false;
                }

                buf.put_u8(0x1);
                buf.put_slice(addr.as_ref());
                buf.put_u16(*port);
            }
            Address::V6(addr, port) => {
                if buf.remaining_mut() < 1 + addr.len() + 2 {
                    return false;
                }

                buf.put_u8(0x4);
                buf.put_slice(addr.as_ref());
                buf.put_u16(*port);
            }
            Address::Name(host, port) => {
                if buf.remaining_mut() < 1 + 2 + host.as_bytes().len() + 2 {
                    return false;
                }

                if host.len() > u16::MAX as usize {
                    return false;
                }

                buf.put_u8(0x3);
                buf.put_u16(host.as_bytes().len() as u16);
                buf.put_slice(host.as_bytes());
                buf.put_u16(*port);
            }
        }

        true
    }

    pub fn write_len(&self) -> usize {
        3 + match self {
            Address::V4(ip, _) => ip.len(),
            Address::V6(ip, _) => ip.len(),
            Address::Name(name, _) => 2 + name.len(),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message<'a> {
    Send(Cow<'a, [u8]>),
    SendTo(Address<'a>, Cow<'a, [u8]>),
}

impl<'a> Message<'a> {
    pub fn parse<'b>(buf: &'b mut &'a [u8]) -> ParseResult<Self> {
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
                *buf = rest;
                Ok(Some(Self::Send(Cow::Borrowed(pkt))))
            }

            1 => {
                let addr = match Address::parse(buf)? {
                    Some(a) => a,
                    None => return Ok(None),
                };

                if buf.remaining() < 2 {
                    return Ok(None);
                }

                let pkt_len = buf.get_u16() as usize;

                let (pkt, rest) = buf.split_at(pkt_len);
                *buf = rest;
                Ok(Some(Self::SendTo(addr, Cow::Borrowed(pkt))))
            }

            v => Err(ParseError::unexpected("Message type", v, "0 or 1")),
        }
    }

    pub fn write(&self, buf: &mut impl BufMut) -> bool {
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
            let mut buf = buf.as_slice();
            let msg1 = Message::parse(&mut buf)
                .expect("To parse 1st msg")
                .expect("Message to be there");
            let msg2 = Message::parse(&mut buf)
                .expect("To parse 2nd msg")
                .expect("Message to be there");
            let msg3 = Message::parse(&mut buf).expect("To parse 3rd msg");

            assert_eq!(
                msg1,
                Message::SendTo(Default::default(), Cow::Borrowed(data))
            );
            assert_eq!(msg2, Message::Send(Cow::Borrowed(data)));
            assert_eq!(msg3, None);
        }
    }
}
