use super::{Condition, Rule};
use std::fmt::Display;

impl Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key)?;
        f.write_str(" ")?;
        match &self.op {
            super::op::Op::Equals(other) => write!(f, "== \"{other}\""),
            super::op::Op::NotEquals(other) => write!(f, "!= \"{other}\""),
            super::op::Op::In(list_name) => write!(f, "in \"{list_name}\""),
            super::op::Op::NotIn(list_name) => write!(f, "!in \"{list_name}\""),
            super::op::Op::RegexMatches(m) => write!(f, "~= /{}/", m),
        }
    }
}

impl Display for Rule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rule at Line #{}", self.line_number)
    }
}
