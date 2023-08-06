use super::op::*;
use crate::client::rule::action::Action;
use anyhow::{bail, Context};
use std::{mem::take, str::FromStr};
use tls_parser::nom::AsChar;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCondition {
    pub key: String,
    pub value: String,
    pub op: Op,
}

#[derive(Copy, Clone, Debug)]
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

enum RuleParseState {
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
    CondValueEnded,
    ActionEnd(Action),
}

#[derive(Debug, PartialEq, Eq)]
pub struct RawRule {
    pub conditions: Vec<RawCondition>,
    pub action: Action,
}

impl RawRule {
    pub fn parse(mut reader: impl Iterator<Item = char>) -> anyhow::Result<Option<Self>> {
        let mut state = RuleParseState::Start;
        let mut conditions = Vec::new();

        while let Some(c) = reader.next() {
            state = match (state, c) {
                (RuleParseState::Start, '}') => {
                    return Ok(None);
                }

                (RuleParseState::Start, c) if !c.is_ascii_whitespace() => {
                    RuleParseState::KeyStarted(c.to_string())
                }

                (RuleParseState::KeyStarted(mut key), b) => {
                    if b.is_ascii_whitespace() {
                        RuleParseState::KeyFinished(key)
                    } else if FIRST_OP_SYMBOLIC_CHARS.contains(b) {
                        RuleParseState::OpStarted {
                            key,
                            op: b.to_string(),
                        }
                    } else if b == ';' {
                        return Ok(Some(Self {
                            conditions,
                            action: (key, String::default())
                                .try_into()
                                .context("Parsing action")?,
                        }));
                    } else {
                        key.push(b);
                        RuleParseState::KeyStarted(key)
                    }
                }

                (RuleParseState::KeyFinished(key), c)
                    if FIRST_OP_ALPHA_CHARS.contains(c) || FIRST_OP_SYMBOLIC_CHARS.contains(c) =>
                {
                    RuleParseState::OpStarted {
                        key,
                        op: c.to_string(),
                    }
                }

                (RuleParseState::KeyFinished(key), ';') => {
                    return Ok(Some(Self {
                        conditions,
                        action: (key, String::default())
                            .try_into()
                            .context("Parsing action")?,
                    }))
                }

                (RuleParseState::OpStarted { key, mut op }, c) => {
                    if c.is_ascii_whitespace() {
                        RuleParseState::OpFinished {
                            key,
                            op: op
                                .parse()
                                .with_context(|| format!("Parsing operator: {op}"))?,
                        }
                    } else {
                        op.push(c.as_char());
                        RuleParseState::OpStarted { key, op }
                    }
                }

                (RuleParseState::OpFinished { key, op }, c) if !c.is_ascii_whitespace() => {
                    if c != '"' {
                        bail!("Expecting \" but got {c:?}");
                    }

                    RuleParseState::ValueStarted {
                        key,
                        op,
                        value: Default::default(),
                        escaping: false,
                    }
                }

                (
                    RuleParseState::ValueStarted {
                        key,
                        op,
                        mut value,
                        escaping: true,
                    },
                    c,
                ) => {
                    value.push(c);
                    RuleParseState::ValueStarted {
                        key,
                        op,
                        value,
                        escaping: false,
                    }
                }

                (
                    RuleParseState::ValueStarted {
                        key,
                        op,
                        mut value,
                        escaping: false,
                    },
                    c,
                ) => match c {
                    '\\' => RuleParseState::ValueStarted {
                        key,
                        op,
                        value,
                        escaping: true,
                    },
                    '"' => match op {
                        AssignOrTest::Assign => RuleParseState::ActionEnd(
                            Action::try_from((key, value)).context("parsing action")?,
                        ),

                        AssignOrTest::Test(op) => {
                            conditions.push(RawCondition { key, value, op });
                            RuleParseState::CondValueEnded
                        }
                    },

                    _ => {
                        value.push(c);
                        RuleParseState::ValueStarted {
                            key,
                            op,
                            value,
                            escaping: false,
                        }
                    }
                },

                (RuleParseState::ActionEnd(action), c) if !c.is_ascii_whitespace() => {
                    if c == ';' {
                        return Ok(Some(Self { conditions, action }));
                    } else {
                        bail!("Expecting ; but got {c:?}");
                    }
                }

                (RuleParseState::CondValueEnded, c) if !c.is_ascii_whitespace() => {
                    if c == ',' {
                        RuleParseState::Start
                    } else {
                        bail!("Expecting , but got {c:?}");
                    }
                }

                (s, _) => s,
            }
        }

        bail!("Unexpected EOF while parsing rule")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RawTable {
    pub name: String,
    pub rules: Vec<RawRule>,
}

#[derive(Debug, PartialEq, Eq)]
enum TableParseState {
    Start,
    NameStarted(String),
    NameEnded(String),
}

impl RawTable {
    pub fn parse(mut reader: impl Iterator<Item = char>) -> anyhow::Result<Option<Self>> {
        let mut state = TableParseState::Start;

        while let Some(c) = reader.next() {
            match (&mut state, c) {
                (TableParseState::Start, c) if !c.is_whitespace() => {
                    state = TableParseState::NameStarted(c.to_string());
                }

                (TableParseState::NameStarted(name), c) => {
                    if c.is_alphanumeric() {
                        name.push(c);
                    } else if c.is_ascii_whitespace() {
                        state = TableParseState::NameEnded(take(name));
                    } else {
                        bail!("Invalid character in the name {name}")
                    }
                }

                (TableParseState::NameEnded(name), '{') => {
                    let mut rules = Vec::new();
                    while let Some(rule) = RawRule::parse(&mut reader).context("parsing rule")? {
                        rules.push(rule);
                    }

                    return Ok(Some(RawTable {
                        name: take(name),
                        rules,
                    }));
                }

                _ => {}
            }
        }

        if state != TableParseState::Start {
            bail!("Expected rules but got nothing");
        }

        Ok(None)
    }
}

pub struct RawProgram {
    pub tables: Vec<RawTable>,
}

impl FromStr for RawProgram {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();
        let mut tables = Vec::new();
        while let Some(table) = RawTable::parse(&mut chars).context("Parsing table")? {
            tables.push(table);
        }

        Ok(Self { tables })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_works() {
        let src = r#"

            main {
                a == "1", b == "v", proxy = "1";
                c == "2", de == "s", reject;
            }

            t1 {

            }

            t2 {
                k == "v", jump = "t1";
            }

        "#;

        let RawProgram { tables } = src.parse().expect("To parse table");

        assert_eq!(
            tables,
            vec![
                RawTable {
                    name: "main".to_string(),
                    rules: vec![
                        RawRule {
                            conditions: vec![
                                RawCondition {
                                    key: "a".to_string(),
                                    value: "1".to_string(),
                                    op: Op::Equals
                                },
                                RawCondition {
                                    key: "b".to_string(),
                                    value: "v".to_string(),
                                    op: Op::Equals
                                },
                            ],
                            action: Action::Proxy("1".to_string())
                        },
                        RawRule {
                            conditions: vec![
                                RawCondition {
                                    key: "c".to_string(),
                                    value: "2".to_string(),
                                    op: Op::Equals
                                },
                                RawCondition {
                                    key: "de".to_string(),
                                    value: "s".to_string(),
                                    op: Op::Equals
                                },
                            ],
                            action: Action::Reject
                        },
                    ]
                },
                RawTable {
                    name: "t1".to_string(),
                    rules: vec![]
                },
                RawTable {
                    name: "t2".to_string(),
                    rules: vec![RawRule {
                        conditions: vec![RawCondition {
                            key: "k".to_string(),
                            value: "v".to_string(),
                            op: Op::Equals
                        },],
                        action: Action::Jump("t1".to_string())
                    }]
                }
            ]
        );
    }
}
