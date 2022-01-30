use anyhow::anyhow;
use std::fmt::{Debug, Display, Formatter, Write};
use std::num::NonZeroU8;
use std::str::FromStr;

#[derive(Copy, Clone, Hash, Eq)]
#[repr(C)]
pub struct CountryCode([NonZeroU8; 2]);

impl CountryCode {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { &*(&self.0 as *const [NonZeroU8; 2] as *const [u8; 2]) }
    }
}

impl Display for CountryCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_char(self.0[0].get().into())?;
        f.write_char(self.0[1].get().into())
    }
}

impl Debug for CountryCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl PartialEq<str> for CountryCode {
    fn eq(&self, other: &str) -> bool {
        other.as_bytes().eq_ignore_ascii_case(self.as_slice())
    }
}

impl PartialEq for CountryCode {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice().eq_ignore_ascii_case(other.as_slice())
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
