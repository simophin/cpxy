use std::borrow::Cow;
use std::fmt::{Debug, Display, Formatter};
use std::io::Write;
use std::str::FromStr;

use anyhow::{bail, Context};
use bytes::Buf;
use either::Either;

use super::message::Record;

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

impl<'a> Record<'a> for Labeled<'a> {
    fn parse(b: &'a [u8]) -> anyhow::Result<(&'a [u8], Self)>
    where
        Self: 'a + Sized,
    {
        let end = b
            .iter()
            .position(|x| *x == 0)
            .context("Finding NULL terminator for Name")?;
        Ok((&b[end + 1..], Self::Buffered(Cow::Borrowed(&b[..end]))))
    }
}

impl<'a> Labeled<'a> {
    pub fn iter(&'a self) -> impl Iterator<Item = &'a str> {
        LabeledIterator(match self {
            Self::Buffered(b) => Either::Left(b.as_ref()),
            Self::Stringed(str) => Either::Right(str),
        })
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
