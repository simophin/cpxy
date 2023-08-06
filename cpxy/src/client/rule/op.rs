use std::str::FromStr;

use super::PropertyValue;
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

pub const FIRST_OP_SYMBOLIC_CHARS: &str = "=!~";
pub const FIRST_OP_ALPHA_CHARS: &str = "i";

impl Op {
    pub fn test(&self, left: &PropertyValue<'_>, right: &str) -> bool {
        match self {
            Self::Equals => left == right,
            Self::NotEquals => left != right,
            Self::Contains => left.contains(right),
            Self::NotContains => !left.contains(right),
            Self::RegexMatches => regex::Regex::new(right).unwrap().is_match(left),
        }
    }
}
