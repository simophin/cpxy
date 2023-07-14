use std::str::FromStr;

use anyhow::bail;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    RegexMatches,
}

impl FromStr for Op {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "==" => Ok(Self::Equals),
            "!=" => Ok(Self::NotEquals),
            "in" => Ok(Self::Contains),
            "!in" => Ok(Self::NotContains),
            "~=" => Ok(Self::RegexMatches),
            _ => bail!("invalid operator: {s}"),
        }
    }
}

pub const FIRST_OP_BYTES: &[u8] = b"=!i~";
