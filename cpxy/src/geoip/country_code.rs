use anyhow::anyhow;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::fmt::{Debug, Display, Formatter};
use std::num::NonZeroU8;
use std::str::FromStr;

#[derive(Copy, Clone, Hash, Eq, SerializeDisplay, DeserializeFromStr, PartialOrd, Ord)]
#[repr(C)]
pub struct CountryCode([NonZeroU8; 2]);

impl CountryCode {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { &*(&self.0 as *const [NonZeroU8; 2] as *const [u8; 2]) }
    }
}

impl AsRef<str> for CountryCode {
    fn as_ref(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(self.as_slice()) }
    }
}

impl Display for CountryCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl Debug for CountryCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl PartialEq<str> for CountryCode {
    fn eq(&self, other: &str) -> bool {
        self.as_ref().eq_ignore_ascii_case(other.as_ref())
    }
}

impl PartialEq for CountryCode {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref().eq_ignore_ascii_case(other.as_ref())
    }
}

impl FromStr for CountryCode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.as_bytes();
        if s.len() != 2 {
            return Err(anyhow!("Must be 2 bytes"));
        }

        Ok(Self([s[0].try_into()?, s[1].try_into()?]))
    }
}
