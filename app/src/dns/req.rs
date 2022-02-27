use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    net::SocketAddr,
    str::FromStr,
};

use anyhow::{bail, Context};
use bit::BitIndex;
use bytes::Buf;
use smallvec::SmallVec;

use crate::shared_udp::IDParser;

#[derive(PartialEq, Eq, Debug)]
pub struct Labeled<'a>(Cow<'a, [u8]>);

impl<'a> Default for Labeled<'a> {
    fn default() -> Self {
        Labeled(Cow::Borrowed(b""))
    }
}

impl<'a> AsRef<[u8]> for Labeled<'a> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<'a> Labeled<'a> {
    pub fn iter(&'a self) -> impl Iterator<Item = &'a str> {
        LabeledIterator(self.0.as_ref())
    }

    pub fn parse(b: &'a [u8]) -> Option<(usize, Labeled<'a>)> {
        let end = b.iter().position(|x| *x == 0)? + 1;
        Some((end, Self(Cow::Borrowed(&b[..end]))))
    }
}

impl FromStr for Labeled<'static> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut buf = Vec::with_capacity(s.as_bytes().len() + 2);
        for seg in s.split('.') {
            if seg.is_empty() {
                bail!("Invalid domain name {s}");
            }
            let seg_bytes = seg.as_bytes();
            let seg_len: u8 = seg_bytes
                .len()
                .try_into()
                .with_context(|| format!("Segment {seg} of domain {s} is more than 255 in size"))?;

            buf.push(seg_len);
            buf.extend_from_slice(seg_bytes);
        }
        buf.push(0);
        Ok(Self(Cow::Owned(buf)))
    }
}

struct LabeledIterator<'a>(&'a [u8]);

impl<'a> Iterator for LabeledIterator<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            return None;
        }

        let len = self.0.get_u8() as usize;
        if self.0.remaining() < len || len == 0 {
            return None;
        }

        let (s, rest) = self.0.split_at(len);
        self.0 = rest;

        match std::str::from_utf8(s) {
            Ok(v) => Some(v),
            Err(e) => {
                log::error!("Invalid UTF-8 string in domain name: {e:?}");
                None
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

pub struct Question<'a> {
    pub domain_name: Labeled<'a>,
    pub t: u16,
    pub class: u16,
}

impl<'a> Question<'a> {
    pub fn parse(mut b: &'a [u8]) -> Option<(usize, Question<'a>)> {
        let (label_offset, domain_name) = Labeled::parse(b)?;
        if b.len() >= label_offset + 4 {
            b.advance(label_offset);
            let t = b.get_u16();
            let class = b.get_u16();
            return Some((
                label_offset + 4,
                Self {
                    domain_name,
                    t,
                    class,
                },
            ));
        }
        None
    }

    pub fn parse_vec(b: &mut &'a [u8], count: usize) -> SmallVec<[Question<'a>; 1]> {
        let mut ret = SmallVec::with_capacity(count);

        for _ in 0..count {
            match Self::parse(*b) {
                Some((offset, v)) => {
                    b.advance(offset);
                    ret.push(v);
                }
                None => break,
            }
        }

        ret
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

pub struct Answer<'a> {
    pub name: Labeled<'a>,
    pub t: u16,
    pub class: u16,
    pub rdata: Cow<'a, [u8]>,
}

impl<'a> Debug for Answer<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Answer")
            .field("name", &self.name)
            .field("t", &self.t)
            .field("class", &self.class)
            .field("rdata_len", &self.rdata.len())
            .finish()
    }
}

impl<'a> Answer<'a> {
    pub fn parse(mut b: &'a [u8]) -> Option<(usize, Answer<'a>)> {
        let (
            offset,
            Question {
                domain_name,
                t,
                class,
            },
        ) = Question::parse(b)?;
        b.advance(offset);
        if b.remaining() < 2 {
            return None;
        }

        let rdata_len = b.get_u16() as usize;
        if b.remaining() < rdata_len {
            return None;
        }
        let rdata = Cow::Borrowed(&b[..rdata_len]);
        Some((
            offset + 2 + rdata_len,
            Self {
                name: domain_name,
                t,
                class,
                rdata,
            },
        ))
    }

    pub fn parse_vec<const N: usize>(b: &mut &'a [u8], count: usize) -> SmallVec<[Answer<'a>; N]> {
        let mut ret = SmallVec::with_capacity(count);

        for _ in 0..count {
            match Self::parse(*b) {
                Some((offset, v)) => {
                    b.advance(offset);
                    ret.push(v);
                }
                None => break,
            }
        }

        ret
    }
}

pub struct Message<'a> {
    pub id: u16,
    header: u16,
    pub questions: SmallVec<[Question<'a>; 1]>,
    pub answers: SmallVec<[Answer<'a>; 2]>,
    pub nsr: SmallVec<[Answer<'a>; 1]>,
    pub ar: SmallVec<[Answer<'a>; 1]>,
}

impl<'a> Debug for Message<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("id", &self.id)
            .field("is_request", &self.is_request())
            .field("opcode", &self.opcode())
            .field("is_aa", &self.is_aa())
            .field("is_truncated", &self.is_truncated())
            .field("recursion_desired", &self.recursion_desired())
            .field("recursion_available", &self.recursion_available())
            .field("response_code", &self.response_code())
            .field("questions", &self.questions)
            .field("answers", &self.answers)
            .field("nsr", &self.nsr)
            .field("ar", &self.ar)
            .finish()
    }
}

impl<'a> Message<'a> {
    pub fn is_request(&self) -> bool {
        self.header.bit(15)
    }

    pub fn opcode(&self) -> u16 {
        self.header.bit_range(11..15)
    }

    pub fn is_aa(&self) -> bool {
        self.header.bit(10)
    }

    pub fn is_truncated(&self) -> bool {
        self.header.bit(9)
    }

    pub fn recursion_desired(&self) -> bool {
        self.header.bit(8)
    }

    pub fn recursion_available(&self) -> bool {
        self.header.bit(7)
    }

    pub fn response_code(&self) -> u16 {
        self.header.bit_range(0..4)
    }

    pub fn parse(mut b: &'a [u8]) -> Option<Message<'a>> {
        if b.len() < 12 {
            return None;
        }

        let id = b.get_u16();
        let header = b.get_u16();
        let qdcount = b.get_u16() as usize;
        let ancount = b.get_u16() as usize;
        let nscount = b.get_u16() as usize;
        let arcount = b.get_u16() as usize;

        Some(Self {
            id,
            header,
            questions: Question::parse_vec(&mut b, qdcount),
            answers: Answer::parse_vec(&mut b, ancount),
            nsr: Answer::parse_vec(&mut b, nscount),
            ar: Answer::parse_vec(&mut b, arcount),
        })
    }
}

impl<'a> IDParser for Message<'a> {
    type IDType = u16;

    fn parse_id(mut buf: &[u8], _: Option<&SocketAddr>) -> anyhow::Result<Self::IDType> {
        if buf.len() < 12 {
            bail!(
                "Invalid len for DNS header, expected at last 12, got: {}",
                buf.len()
            );
        }

        Ok(buf.get_u16())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn label_works() {
        let domain = "www.163.com";
        let l: Labeled<'_> = domain.parse().unwrap();
        assert_eq!(domain, l.to_string().as_str());

        let mut buf = Vec::new();
        buf.extend_from_slice(l.as_ref());
        buf.extend_from_slice(b"hello, world");
        let (offset, parsed) = Labeled::parse(&buf).unwrap();
        assert_eq!(parsed, l);
        assert_eq!(&buf[offset..], b"hello, world");
    }
}
