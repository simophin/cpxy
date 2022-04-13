use std::{fmt::Display, str::FromStr};

#[derive(Debug, Clone)]
pub struct Pattern(regex::Regex);

impl FromStr for Pattern {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(regex::Regex::new(s)?))
    }
}

impl Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl Pattern {
    pub fn matches(&self, needle: &str) -> bool {
        self.0.is_match(needle)
    }
}
