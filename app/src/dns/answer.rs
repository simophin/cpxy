use std::{
    borrow::Cow,
    io::Write,
    net::{Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use anyhow::{bail, Context};
use bit::BitIndex;
use byteorder::{BigEndian, WriteBytesExt};
use bytes::Buf;

use super::{
    labeled::Labeled,
    message::{Class, Record, Type, CLASS_IN, TYPE_A, TYPE_AAAA},
};

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

impl<'a> Record<'a> for AnswerName<'a> {
    fn parse(mut b: &'a [u8]) -> anyhow::Result<(&'a [u8], AnswerName<'a>)> {
        if b.len() < 2 {
            bail!("Insufficient buffer for AnswerName");
        }

        let mut ptr = u16::from_be_bytes([b[0], b[1]]);
        if ptr.bit(14) && ptr.bit(15) {
            ptr.set_bit(14, false);
            ptr.set_bit(15, false);
            b.advance(2);
            Ok((b, AnswerName::Pointer(ptr)))
        } else {
            Labeled::parse(b).map(|(buf, labeled)| (buf, AnswerName::QName(labeled)))
        }
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
}

#[derive(Debug)]
pub struct Answer<'a> {
    pub name: AnswerName<'a>,
    pub ttl: Duration,
    pub record: AnswerRecord<'a>,
}

impl<'a> Answer<'a> {
    pub fn to_writer(&self, w: &mut impl Write) -> anyhow::Result<()> {
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
    fn parse(b: &'a [u8]) -> anyhow::Result<(&'a [u8], Answer<'a>)> {
        let (mut b, name) = AnswerName::parse(b)?;

        if b.remaining() < 10 {
            bail!("Insufficient buffer for answer");
        }

        let t = b.get_u16();
        let class = b.get_u16();
        let ttl = b.get_u32();
        let rdata_len = b.get_u16() as usize;
        if b.remaining() < rdata_len {
            bail!("Insufficient Record");
        }
        let rdata = Cow::Borrowed(&b[..rdata_len]);
        Ok((
            b,
            Self {
                name,
                ttl: Duration::from_secs(ttl as u64),
                record: AnswerRecord::new(t, class, rdata)?,
            },
        ))
    }
}
