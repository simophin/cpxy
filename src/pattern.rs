use std::{fmt::Display, str::FromStr, sync::Arc};

#[derive(Debug, Clone)]
pub struct Pattern(Arc<str>, regex::Regex);

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Pattern {}

impl FromStr for Pattern {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let r = regex::Regex::new(s)?;
        Ok(Self(s.into(), r))
    }
}

impl Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_ref())
    }
}

impl Pattern {
    pub fn matches(&self, needle: &str) -> bool {
        self.1.is_match(needle)
    }
}
