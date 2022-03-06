use std::fmt::Debug;
use std::fmt::Formatter;
use std::io::Write;
use std::net::IpAddr;
use std::time::Duration;

use anyhow::bail;
use bit::BitIndex;
use byteorder::{BigEndian, WriteBytesExt};
use bytes::Buf;
use smallvec::SmallVec;

use super::answer::{Answer, AnswerRecord};
use super::question::Question;

pub trait Record<'a> {
    fn parse(b: &'a [u8]) -> anyhow::Result<(&'a [u8], Self)>
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
            let (r, v) = Self::parse(*b)?;
            *b = r;
            ret.push(v);
        }

        Ok(ret)
    }
}

pub type Type = u16;
pub const TYPE_A: Type = 1;
pub const TYPE_AAAA: Type = 28;

pub type Class = u16;
pub const CLASS_IN: Class = 1;

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

impl<'a> Record<'a> for Message<'a> {
    fn parse(mut b: &'a [u8]) -> anyhow::Result<(&'a [u8], Self)>
    where
        Self: 'a + Sized,
    {
        if b.len() < 12 {
            bail!("Invalid header length");
        }

        let id = b.get_u16();
        let header = Header(b.get_u16());
        let qdcount = b.get_u16() as usize;
        let ancount = b.get_u16() as usize;
        let nscount = b.get_u16() as usize;
        let arcount = b.get_u16() as usize;

        let questions = Question::parse_vec(&mut b, qdcount)?;
        let answers = Answer::parse_vec(&mut b, ancount)?;
        let nsr = Answer::parse_vec(&mut b, nscount)?;
        let ar = Answer::parse_vec(&mut b, arcount)?;

        Ok((
            b,
            Self {
                id,
                header,
                questions,
                answers,
                nsr,
                ar,
            },
        ))
    }
}

impl<'a> Message<'a> {
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

    use crate::{dns::labeled::Labeled, io::UdpSocket};

    use super::*;

    #[test]
    fn label_works() {
        let domain = "www.163.com";
        let l: Labeled<'_> = domain.parse().unwrap();
        assert_eq!(domain, l.to_string().as_str());

        let mut buf = Vec::new();
        l.to_writer(&mut buf).unwrap();
        buf.extend_from_slice(b"hello, world");
        let (buf, parsed) = Labeled::parse(&buf).unwrap();
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
