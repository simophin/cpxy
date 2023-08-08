use derive_more::Deref;
use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

#[derive(Deref, Clone)]
pub struct Regex(regex::Regex);

impl FromStr for Regex {
    type Err = <regex::Regex as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

impl Debug for Regex {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for Regex {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq for Regex {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Regex {}
