use std::io::Write;

use anyhow::bail;
use byteorder::{BigEndian, WriteBytesExt};
use bytes::Buf;
use std::fmt::{Debug, Formatter};

use super::{
    labeled::Labeled,
    message::{Class, Record, Type},
};

pub struct Question<'a> {
    pub domain_name: Labeled<'a>,
    pub t: Type,
    pub class: Class,
}

impl<'a> Question<'a> {
    pub fn to_writer(&self, w: &mut impl Write) -> anyhow::Result<()> {
        self.domain_name.to_writer(w)?;
        w.write_u16::<BigEndian>(self.t)?;
        w.write_u16::<BigEndian>(self.class)?;
        Ok(())
    }
}

impl<'a> Record<'a> for Question<'a> {
    fn parse(b: &'a [u8]) -> anyhow::Result<(&'a [u8], Question<'a>)> {
        let (mut b, domain_name) = Labeled::parse(b)?;

        if b.len() < 4 {
            bail!("Insufficient buffer for question");
        }

        let t = b.get_u16();
        let class = b.get_u16();
        Ok((
            b,
            Self {
                domain_name,
                t,
                class,
            },
        ))
    }
}

impl<'a> Debug for Question<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Question")
            .field("domain_name", &self.domain_name)
            .field("t", &self.t)
            .field("class", &self.class)
            .finish()
    }
}
