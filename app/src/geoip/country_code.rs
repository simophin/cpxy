use anyhow::anyhow;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt::{Debug, Display, Formatter, Write};
use std::num::NonZeroU8;
use std::str::FromStr;

#[derive(Copy, Clone, Hash, Eq)]
#[repr(C)]
pub struct CountryCode([NonZeroU8; 2]);

impl Serialize for CountryCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str().as_ref())
    }
}

struct CountryCodeVisitor;

impl Visitor<'_> for CountryCodeVisitor {
    type Value = CountryCode;

    fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
        formatter.write_str("Expected string for CountryCode")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        CountryCode::from_str(v).map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}

impl<'de> Deserialize<'de> for CountryCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(CountryCodeVisitor)
    }
}

impl CountryCode {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { &*(&self.0 as *const [NonZeroU8; 2] as *const [u8; 2]) }
    }

    pub fn as_str(&self) -> Cow<str> {
        String::from_utf8_lossy(self.as_slice())
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
