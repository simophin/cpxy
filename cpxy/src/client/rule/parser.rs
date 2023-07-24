use super::op::*;
use anyhow::{bail, Context};
use std::str::FromStr;
use tls_parser::nom::AsChar;

pub struct Condition {
    pub key: String,
    pub value: String,
    pub op: Op,
}

#[derive(Default)]
pub struct Action {
    pub what: String,
    pub how: String,
}

#[derive(Copy, Clone)]
enum AssignOrTest {
    Assign,
    Test(Op),
}

impl FromStr for AssignOrTest {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "=" => Ok(Self::Assign),
            _ => Ok(Self::Test(s.parse()?)),
        }
    }
}

enum ParseState {
    Start,
    KeyStarted(String),
    KeyFinished(String),
    OpStarted {
        key: String,
        op: String,
    },
    OpFinished {
        key: String,
        op: AssignOrTest,
    },
    ValueStarted {
        key: String,
        op: AssignOrTest,

        value: String,
        escaping: bool,
    },
    ActionEnd(Action),
}

pub struct RawRule {
    pub conditions: Vec<Condition>,
    pub action: Action,
}

impl RawRule {
    pub fn parse<E: std::error::Error + Send + Sync>(
        reader: &mut impl Iterator<Item = Result<char, E>>,
    ) -> anyhow::Result<Self> {
        let mut state = ParseState::Start;
        let mut conditions = Vec::new();

        while let Some(c) = reader.next() {
            match (&mut state, c.context("reading char")?) {
                (ParseState::Start, c) if !c.is_ascii_whitespace() => {
                    state = ParseState::KeyStarted(c.to_string());
                }

                (ParseState::KeyStarted(cond), b) => {
                    if b.is_ascii_whitespace() {
                        state = ParseState::KeyFinished(std::mem::take(cond));
                    } else if FIRST_OP_CHARS.contains(b) {
                        state = ParseState::OpStarted {
                            key: std::mem::take(cond),
                            op: b.to_string(),
                        };
                    } else {
                        cond.push(b);
                    }
                }

                (ParseState::KeyFinished(cond), c) if FIRST_OP_CHARS.contains(c) => {
                    state = ParseState::OpStarted {
                        key: std::mem::take(cond),
                        op: c.to_string(),
                    };
                }

                (ParseState::OpStarted { key: cond, op }, c) => {
                    if c.is_ascii_whitespace() {
                        state = ParseState::OpFinished {
                            key: std::mem::take(cond),
                            op: op
                                .parse()
                                .with_context(|| format!("Parsing operator: {cond}"))?,
                        };
                    } else {
                        op.push(c.as_char());
                    }
                }

                (ParseState::OpFinished { key: cond_key, op }, c) if !c.is_ascii_whitespace() => {
                    if c != '"' {
                        bail!("Expecting \" but got {c:?}");
                    }

                    state = ParseState::ValueStarted {
                        key: std::mem::take(cond_key),
                        op: *op,
                        value: Default::default(),
                        escaping: false,
                    };
                }

                (
                    ParseState::ValueStarted {
                        key,
                        op,
                        value,
                        escaping,
                    },
                    c,
                ) => {
                    if *escaping {
                        value.push(c);
                        *escaping = false;
                        continue;
                    }

                    match c {
                        '\\' => *escaping = true,
                        '"' => match op {
                            AssignOrTest::Assign => {
                                state = ParseState::ActionEnd(Action {
                                    what: std::mem::take(key),
                                    how: std::mem::take(value),
                                });
                            }

                            AssignOrTest::Test(op) => {
                                conditions.push(Condition {
                                    key: std::mem::take(key),
                                    value: std::mem::take(value),
                                    op: *op,
                                });

                                state = ParseState::Start;
                            }
                        },

                        _ => value.push(c),
                    };
                }

                (ParseState::ActionEnd(a), c) if !c.is_ascii_whitespace() => {
                    if c == ';' {
                        return Ok(Self {
                            conditions,
                            action: std::mem::take(a),
                        });
                    } else {
                        bail!("Expecting ; but got {c:?}");
                    }
                }

                _ => {}
            }
        }

        bail!("Unexpected EOF while parsing rule")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_works() {}
}
