use std::fmt::Formatter;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;
use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    str::FromStr,
};

use anyhow::{bail, Context};
use bit::BitIndex;
use byteorder::{BigEndian, WriteBytesExt};
use bytes::Buf;
use either::Either;
use smallvec::SmallVec;

#[derive(Eq)]
pub enum Labeled<'a> {
    Buffered(Cow<'a, [u8]>),
    Stringed(Cow<'a, str>),
}

impl<'a> Default for Labeled<'a> {
    fn default() -> Self {
        Labeled::Buffered(Cow::Borrowed(b""))
    }
}

impl<'a> Debug for Labeled<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self, f)
    }
}

impl<'a> Labeled<'a> {
    pub fn iter(&'a self) -> impl Iterator<Item = &'a str> {
        LabeledIterator(match self {
            Self::Buffered(b) => Either::Left(b.as_ref()),
            Self::Stringed(str) => Either::Right(str),
        })
    }

    pub fn parse(b: &'a [u8]) -> anyhow::Result<Option<(&'a [u8], Labeled<'a>)>> {
        let end = b
            .iter()
            .position(|x| *x == 0)
            .context("Finding NULL terminator for Name")?;
        Ok(Some((
            &b[end + 1..],
            Self::Buffered(Cow::Borrowed(&b[..end])),
        )))
    }

    fn check_string(domain: &str) -> anyhow::Result<&str> {
        for seg in domain.split('.') {
            if seg.is_empty() {
                bail!("Invalid domain name {domain}");
            }

            if seg.as_bytes().len() > 255 {
                bail!("Segment {seg} of domain {domain} is more than 255 in length")
            }
        }
        Ok(domain)
    }
}

impl<'a> TryFrom<&'a str> for Labeled<'a> {
    type Error = anyhow::Error;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        let s = Self::check_string(value)?;
        Ok(Self::Stringed(Cow::Borrowed(s)))
    }
}

impl<'a> PartialEq for Labeled<'a> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Buffered(l0), Self::Buffered(r0)) => l0 == r0,
            (Self::Stringed(l0), Self::Stringed(r0)) => l0 == r0,
            (lhs, rhs) => lhs.iter().eq(rhs.iter()),
        }
    }
}

impl FromStr for Labeled<'static> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::Stringed(Cow::Owned(
            Self::check_string(s)?.to_string(),
        )))
    }
}

impl<'a> Labeled<'a> {
    pub fn to_writer(&self, w: &mut impl Write) -> anyhow::Result<()> {
        match self {
            Self::Buffered(buf) => {
                w.write_all(buf.as_ref())?;
            }
            _ => {
                for seg in self.iter() {
                    let seg_bytes = seg.as_bytes();
                    w.write(&[seg_bytes.len() as u8])?;
                    w.write_all(seg_bytes)?;
                }
                w.write(&[0])?;
            }
        }
        Ok(())
    }
}

struct LabeledIterator<'a>(Either<&'a [u8], &'a str>);

impl<'a> Iterator for LabeledIterator<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0 {
            Either::Right(s) => {
                if s.is_empty() {
                    return None;
                }

                let (curr, rest) = match s.find('.') {
                    Some(index) => (&s[..index], &s[index + 1..]),
                    None => (s, ""),
                };

                self.0 = Either::Right(rest);
                Some(curr)
            }

            Either::Left(mut buf) => {
                if buf.is_empty() {
                    return None;
                }

                let len = buf.get_u8() as usize;
                if buf.remaining() < len || len == 0 {
                    return None;
                }

                let (s, buf) = buf.split_at(len);
                self.0 = Either::Left(buf);
                match std::str::from_utf8(s) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        log::error!("Invalid UTF-8 string in domain name: {e:?}");
                        None
                    }
                }
            }
        }
    }
}

impl<'a> Display for Labeled<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut is_first = true;
        for label in self.iter() {
            if is_first {
                is_first = false;
            } else {
                f.write_str(".")?;
            }
            f.write_str(label)?;
        }
        Ok(())
    }
}

trait Record<'a> {
    fn parse(b: &'a [u8]) -> anyhow::Result<Option<(&'a [u8], Self)>>
    where
        Self: 'a + Sized;

    fn parse_vec<const N: usize>(
        b: &mut &'a [u8],
        count: usize,
    ) -> anyhow::Result<SmallVec<[Self; N]>>
    where
        Self: 'a + Sized,
    {
        let mut ret = SmallVec::with_capacity(count);

        for _ in 0..count {
            match Self::parse(*b)? {
                Some((r, v)) => {
                    *b = r;
                    ret.push(v);
                }
                None => bail!("Unexpected incomplete element"),
            }
        }

        Ok(ret)
    }
}

pub type Type = u16;
pub const TYPE_A: Type = 1;
pub const TYPE_AAAA: Type = 28;

pub type Class = u16;
pub const CLASS_IN: Class = 1;

pub struct Question<'a> {
    pub domain_name: Labeled<'a>,
    pub t: Type,
    pub class: Class,
}

impl<'a> Question<'a> {
    fn to_writer(&self, w: &mut impl Write) -> anyhow::Result<()> {
        self.domain_name.to_writer(w)?;
        w.write_u16::<BigEndian>(self.t)?;
        w.write_u16::<BigEndian>(self.class)?;
        Ok(())
    }
}

impl<'a> Record<'a> for Question<'a> {
    fn parse(b: &'a [u8]) -> anyhow::Result<Option<(&'a [u8], Question<'a>)>> {
        let (mut b, domain_name) = match Labeled::parse(b)? {
            Some(v) => v,
            None => return Ok(None),
        };
        if b.len() < 4 {
            return Ok(None);
        }

        let t = b.get_u16();
        let class = b.get_u16();
        Ok(Some((
            b,
            Self {
                domain_name,
                t,
                class,
            },
        )))
    }
}

impl<'a> Debug for Question<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Question")
            .field("domain_name", &self.domain_name)
            .field("t", &self.t)
            .field("class", &self.class)
            .finish()
    }
}

#[derive(Debug)]
pub enum AnswerRecord<'a> {
    A(Ipv4Addr),
    AAAA(Ipv6Addr),
    Other(Type, Class, Cow<'a, [u8]>),
}

impl<'a> AnswerRecord<'a> {
    fn record_type(&self) -> Type {
        match self {
            Self::A(_) => TYPE_A,
            Self::AAAA(_) => TYPE_AAAA,
            Self::Other(t, _, _) => *t,
        }
    }

    fn record_class(&self) -> Class {
        match self {
            Self::A(_) | Self::AAAA(_) => CLASS_IN,
            Self::Other(_, c, _) => *c,
        }
    }

    fn new(t: Type, c: Class, data: Cow<'a, [u8]>) -> anyhow::Result<Self> {
        Ok(match (t, c) {
            (TYPE_A, CLASS_IN) if data.len() == 4 => {
                let data: [u8; 4] = data.as_ref().try_into()?;
                Self::A(Ipv4Addr::from(data))
            }
            (TYPE_AAAA, CLASS_IN) if data.len() == 16 => {
                let data: [u8; 16] = data.as_ref().try_into()?;
                Self::AAAA(Ipv6Addr::from(data))
            }
            _ => Self::Other(t, c, data),
        })
    }

    fn to_writer(&self, w: &mut impl Write) -> anyhow::Result<()> {
        match self {
            Self::A(addr) => {
                w.write_u16::<BigEndian>(4)?;
                w.write_all(addr.octets().as_ref())?;
            }
            Self::AAAA(addr) => {
                w.write_u16::<BigEndian>(16)?;
                w.write_all(addr.octets().as_ref())?;
            }
            Self::Other(_, _, data) => {
                w.write_u16::<BigEndian>(data.len().try_into().context("Record data too big")?)?;
                w.write_all(data.as_ref())?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum AnswerName<'a> {
    QName(Labeled<'a>),
    Pointer(u16),
}

impl<'a> Default for AnswerName<'a> {
    fn default() -> Self {
        AnswerName::QName(Default::default())
    }
}

impl<'a> AnswerName<'a> {
    fn to_writer(&self, w: &mut impl Write) -> anyhow::Result<()> {
        match self {
            Self::QName(l) => l.to_writer(w),
            Self::Pointer(u) => {
                w.write_u16::<BigEndian>(*u | (0x11 << 14))?;
                Ok(())
            }
        }
    }

    fn parse(mut b: &'a [u8]) -> anyhow::Result<Option<(&'a [u8], AnswerName<'a>)>> {
        if b.len() < 2 {
            return Ok(None);
        }

        let mut ptr = u16::from_be_bytes([b[0], b[1]]);
        if ptr.bit(14) && ptr.bit(15) {
            ptr.set_bit(14, false);
            ptr.set_bit(15, false);
            b.advance(2);
            Ok(Some((b, AnswerName::Pointer(ptr))))
        } else {
            match Labeled::parse(b) {
                Ok(Some((b, v))) => Ok(Some((b, AnswerName::QName(v)))),
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            }
        }
    }
}

#[derive(Debug)]
pub struct Answer<'a> {
    pub name: AnswerName<'a>,
    pub ttl: Duration,
    pub record: AnswerRecord<'a>,
}

impl<'a> Answer<'a> {
    fn to_writer(&self, w: &mut impl Write) -> anyhow::Result<()> {
        let ttl: u32 = self.ttl.as_secs().try_into().context("Excessive TTL")?;
        self.name.to_writer(w)?;
        w.write_u16::<BigEndian>(self.record.record_type())?;
        w.write_u16::<BigEndian>(self.record.record_class())?;
        w.write_u32::<BigEndian>(ttl)?;
        self.record.to_writer(w)?;
        Ok(())
    }
}

impl<'a> Record<'a> for Answer<'a> {
    fn parse(b: &'a [u8]) -> anyhow::Result<Option<(&'a [u8], Answer<'a>)>> {
        let (mut b, name) = match AnswerName::parse(b)? {
            Some(v) => v,
            None => return Ok(None),
        };

        if b.remaining() < 10 {
            return Ok(None);
        }

        let t = b.get_u16();
        let class = b.get_u16();
        let ttl = b.get_u32();
        let rdata_len = b.get_u16() as usize;
        if b.remaining() < rdata_len {
            return Ok(None);
        }
        let rdata = Cow::Borrowed(&b[..rdata_len]);
        Ok(Some((
            b,
            Self {
                name,
                ttl: Duration::from_secs(ttl as u64),
                record: AnswerRecord::new(t, class, rdata)?,
            },
        )))
    }
}

pub type OpCode = u8;
pub const OPCODE_REQUEST: OpCode = 0;

pub type ResponseCode = u8;
pub const RESPONSE_CODE_SUCCESS: ResponseCode = 0;
pub const RESPONSE_CODE_INVALID_FORMAT: ResponseCode = 1;
pub const RESPONSE_CODE_SERVER_FAILURE: ResponseCode = 2;

pub struct Header(u16);

impl Header {
    const fn new_request(opcode: OpCode) -> Self {
        Self((opcode as u16) << 11)
    }

    const fn new_response(response_code: ResponseCode) -> Self {
        Self((response_code as u16) | (1 << 15))
    }

    pub fn is_request(&self) -> bool {
        !self.0.bit(15)
    }

    pub fn opcode(&self) -> OpCode {
        self.0.bit_range(11..15) as u8
    }

    pub fn is_aa(&self) -> bool {
        self.0.bit(10)
    }

    pub fn is_truncated(&self) -> bool {
        self.0.bit(9)
    }

    pub fn recursion_desired(&self) -> bool {
        self.0.bit(8)
    }

    pub fn recursion_available(&self) -> bool {
        self.0.bit(7)
    }

    pub fn response_code(&self) -> u16 {
        self.0.bit_range(0..4)
    }
}

impl Debug for Header {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Header")
            .field("is_request", &self.is_request())
            .field("opcode", &self.opcode())
            .field("is_aa", &self.is_aa())
            .field("is_truncated", &self.is_truncated())
            .field("recursion_desired", &self.recursion_desired())
            .field("recursion_available", &self.recursion_available())
            .field("response_code", &self.response_code())
            .finish()
    }
}

#[derive(Debug)]
pub struct Message<'a> {
    pub id: u16,
    pub header: Header,
    pub questions: SmallVec<[Question<'a>; 1]>,
    pub answers: SmallVec<[Answer<'a>; 2]>,
    pub nsr: SmallVec<[Answer<'a>; 1]>,
    pub ar: SmallVec<[Answer<'a>; 1]>,
}

impl<'a> Message<'a> {
    pub fn parse(mut b: &'a [u8]) -> anyhow::Result<Option<Message<'a>>> {
        if b.len() < 12 {
            return Ok(None);
        }

        let id = b.get_u16();
        let header = Header(b.get_u16());
        let qdcount = b.get_u16() as usize;
        let ancount = b.get_u16() as usize;
        let nscount = b.get_u16() as usize;
        let arcount = b.get_u16() as usize;

        Ok(Some(Self {
            id,
            header,
            questions: Question::parse_vec(&mut b, qdcount)?,
            answers: Answer::parse_vec(&mut b, ancount)?,
            nsr: Answer::parse_vec(&mut b, nscount)?,
            ar: Answer::parse_vec(&mut b, arcount)?,
        }))
    }

    fn write_headers(
        w: &mut impl Write,
        id: u16,
        header: u16,
        qcount: u16,
        ancount: u16,
        nscount: u16,
        arcount: u16,
    ) -> anyhow::Result<()> {
        w.write_u16::<BigEndian>(id)?;
        w.write_u16::<BigEndian>(header)?;
        w.write_u16::<BigEndian>(qcount)?;
        w.write_u16::<BigEndian>(ancount)?;
        w.write_u16::<BigEndian>(nscount)?;
        w.write_u16::<BigEndian>(arcount)?;
        Ok(())
    }

    pub fn new_resolve_request(w: &mut impl Write, id: u16, domain: &str) -> anyhow::Result<()> {
        let q = Question {
            domain_name: domain.try_into()?,
            t: TYPE_A,
            class: CLASS_IN,
        };

        Self::write_headers(w, id, Header::new_request(OPCODE_REQUEST).0, 1, 0, 0, 0)?;
        q.to_writer(w)?;
        Ok(())
    }

    pub fn new_resolve_answer(
        w: &mut impl Write,
        id: u16,
        ttl: Duration,
        addr: &IpAddr,
    ) -> anyhow::Result<()> {
        Self::write_headers(
            w,
            id,
            Header::new_response(RESPONSE_CODE_SUCCESS).0,
            0,
            1,
            0,
            0,
        )?;

        Answer {
            name: Default::default(),
            ttl,
            record: match addr {
                IpAddr::V4(addr) => AnswerRecord::A(*addr),
                IpAddr::V6(addr) => AnswerRecord::AAAA(*addr),
            },
        }
        .to_writer(w)
    }
}

#[cfg(test)]
mod test {
    use smol::block_on;
    use smol_timeout::TimeoutExt;

    use crate::io::UdpSocket;

    use super::*;

    #[test]
    fn label_works() {
        let domain = "www.163.com";
        let l: Labeled<'_> = domain.parse().unwrap();
        assert_eq!(domain, l.to_string().as_str());

        let mut buf = Vec::new();
        l.to_writer(&mut buf).unwrap();
        buf.extend_from_slice(b"hello, world");
        let (buf, parsed) = Labeled::parse(&buf).unwrap().unwrap();
        assert_eq!(parsed, l);
        assert_eq!(buf, b"hello, world");
    }

    #[test]
    fn resolve_works() {
        std::env::set_var("RUST_LOG", "debug");
        let _ = env_logger::try_init();
        block_on(async move {
            let socket = UdpSocket::bind(true).await.unwrap();
            let mut buf = Vec::with_capacity(65536);
            Message::new_resolve_request(&mut buf, 1, "www.google.com").unwrap();
            socket.send_to(&buf, "8.8.8.8:53").await.unwrap();

            buf.resize(buf.capacity(), 0);
            let (len, addr) = socket
                .recv_from(&mut buf)
                .timeout(Duration::from_secs(5))
                .await
                .unwrap()
                .unwrap();

            buf.resize(len, 0);
            log::info!("Received {len} bytes from {addr}");
            let res = Message::parse(&buf).unwrap();
            log::info!("Received {res:?} from {addr}");
        });
    }
}
