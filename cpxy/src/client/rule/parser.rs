use super::op::*;
use anyhow::{bail, Context};
use std::{mem::take, str::FromStr};
use tls_parser::nom::AsChar;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub key: String,
    pub value: String,
    pub op: Op,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Action {
    pub what: String,
    pub how: String,
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
    pub conditions: Vec<Condition>,
    pub action: Action,
}

impl RawRule {
    pub fn parse(mut reader: impl Iterator<Item = char>) -> anyhow::Result<Option<Self>> {
        let mut state = RuleParseState::Start;
        let mut conditions = Vec::new();

        while let Some(c) = reader.next() {
            match (&mut state, c) {
                (RuleParseState::Start, '}') => {
                    return Ok(None);
                }

                (RuleParseState::Start, c) if !c.is_ascii_whitespace() => {
                    state = RuleParseState::KeyStarted(c.to_string());
                }

                (RuleParseState::KeyStarted(cond), b) => {
                    if b.is_ascii_whitespace() {
                        state = RuleParseState::KeyFinished(take(cond));
                    } else if FIRST_OP_SYMBOLIC_CHARS.contains(b) {
                        state = RuleParseState::OpStarted {
                            key: take(cond),
                            op: b.to_string(),
                        };
                    } else {
                        cond.push(b);
                    }
                }

                (RuleParseState::KeyFinished(cond), c)
                    if FIRST_OP_ALPHA_CHARS.contains(c) || FIRST_OP_SYMBOLIC_CHARS.contains(c) =>
                {
                    state = RuleParseState::OpStarted {
                        key: take(cond),
                        op: c.to_string(),
                    };
                }

                (RuleParseState::OpStarted { key: cond, op }, c) => {
                    if c.is_ascii_whitespace() {
                        state = RuleParseState::OpFinished {
                            key: take(cond),
                            op: op
                                .parse()
                                .with_context(|| format!("Parsing operator: {op}"))?,
                        };
                    } else {
                        op.push(c.as_char());
                    }
                }

                (RuleParseState::OpFinished { key: cond_key, op }, c)
                    if !c.is_ascii_whitespace() =>
                {
                    if c != '"' {
                        bail!("Expecting \" but got {c:?}");
                    }

                    state = RuleParseState::ValueStarted {
                        key: take(cond_key),
                        op: *op,
                        value: Default::default(),
                        escaping: false,
                    };
                }

                (
                    RuleParseState::ValueStarted {
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
                                state = RuleParseState::ActionEnd(Action {
                                    what: take(key),
                                    how: take(value),
                                });
                            }

                            AssignOrTest::Test(op) => {
                                conditions.push(Condition {
                                    key: take(key),
                                    value: take(value),
                                    op: *op,
                                });

                                state = RuleParseState::CondValueEnded;
                            }
                        },

                        _ => value.push(c),
                    };
                }

                (RuleParseState::ActionEnd(a), c) if !c.is_ascii_whitespace() => {
                    if c == ';' {
                        return Ok(Some(Self {
                            conditions,
                            action: take(a),
                        }));
                    } else {
                        bail!("Expecting ; but got {c:?}");
                    }
                }

                (RuleParseState::CondValueEnded, c) if !c.is_ascii_whitespace() => {
                    if c == ',' {
                        state = RuleParseState::Start;
                    } else {
                        bail!("Expecting , but got {c:?}");
                    }
                }

                _ => {}
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
                a == "1", b == "v", action = "accept";
                c == "2", de == "s", action = "reject";
            }

            t1 {

            }

            t2 {
                k == "v", a = "block";
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
                                Condition {
                                    key: "a".to_string(),
                                    value: "1".to_string(),
                                    op: Op::Equals
                                },
                                Condition {
                                    key: "b".to_string(),
                                    value: "v".to_string(),
                                    op: Op::Equals
                                },
                            ],
                            action: Action {
                                what: "action".to_string(),
                                how: "accept".to_string()
                            }
                        },
                        RawRule {
                            conditions: vec![
                                Condition {
                                    key: "c".to_string(),
                                    value: "2".to_string(),
                                    op: Op::Equals
                                },
                                Condition {
                                    key: "de".to_string(),
                                    value: "s".to_string(),
                                    op: Op::Equals
                                },
                            ],
                            action: Action {
                                what: "action".to_string(),
                                how: "reject".to_string()
                            }
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
                        conditions: vec![Condition {
                            key: "k".to_string(),
                            value: "v".to_string(),
                            op: Op::Equals
                        },],
                        action: Action {
                            what: "a".to_string(),
                            how: "block".to_string()
                        }
                    }]
                }
            ]
        );
    }
}
