use crate::regex::Regex;
use anyhow::{bail, Context};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Equals(String),
    NotEquals(String),
    In(String),
    NotIn(String),
    RegexMatches(Regex),
}

impl<O: AsRef<str>, V: AsRef<str> + Into<String>> TryFrom<(O, V)> for Op {
    type Error = anyhow::Error;

    fn try_from((op, value): (O, V)) -> Result<Self, Self::Error> {
        match op.as_ref() {
            "==" => Ok(Self::Equals(value.into())),
            "!=" => Ok(Self::NotEquals(value.into())),
            "in" => Ok(Self::In(value.into())),
            "!in" => Ok(Self::NotIn(value.into())),
            "~=" => Ok(Self::RegexMatches(
                value.as_ref().parse().context("Parsing regex")?,
            )),
            _ => bail!(
                "invalid operator: {} with value {}",
                op.as_ref(),
                value.as_ref()
            ),
        }
    }
}

pub const FIRST_OP_SYMBOLIC_CHARS: &str = "=!~";
pub const FIRST_OP_ALPHA_CHARS: &str = "i";
