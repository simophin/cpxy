use super::op::*;
use super::{line::ColRowCounter, Action, Condition, Program, Rule, Table};
use anyhow::{bail, Context};
use std::{mem::take, str::FromStr};
use tls_parser::nom::AsChar;

#[derive(Clone, Debug)]
enum AssignOrTest {
    Assign,
    TestOp(String),
}

impl FromStr for AssignOrTest {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "=" => Ok(Self::Assign),
            _ => Ok(Self::TestOp(s.to_string())),
        }
    }
}

#[derive(Debug)]
struct OpStartedState {
    key: String,
    op: String,
}

#[derive(Debug)]
struct OpFinishedState {
    key: String,
    op: AssignOrTest,
}

#[derive(Debug)]
struct ValueStartedState {
    key: String,
    op: AssignOrTest,

    value: String,
    escaping: bool,
}

#[derive(Debug)]
enum RuleParseState {
    Start,
    CommentStarted,
    KeyStarted(String),
    KeyFinished(String),
    OpStarted(OpStartedState),
    OpFinished(OpFinishedState),
    ValueStarted(ValueStartedState),
    CondValueEnded,
    ActionEnd(Action),
}

impl Rule {
    pub fn parse(
        reader: &mut ColRowCounter<impl Iterator<Item = char>>,
    ) -> anyhow::Result<Option<Self>> {
        let mut state = RuleParseState::Start;
        let mut line_number = 1usize;
        let mut conditions = Vec::new();

        while let Some(c) = reader.next() {
            state = match (state, c) {
                (RuleParseState::Start, '}') => {
                    return Ok(None);
                }

                (RuleParseState::Start, '#') => RuleParseState::CommentStarted,

                (RuleParseState::Start, c) if !c.is_ascii_whitespace() => {
                    line_number = reader.line();
                    RuleParseState::KeyStarted(c.to_string())
                }

                (RuleParseState::CommentStarted, '\n') => RuleParseState::Start,

                (RuleParseState::KeyStarted(mut key), b) => {
                    if b.is_ascii_whitespace() {
                        RuleParseState::KeyFinished(key)
                    } else if FIRST_OP_SYMBOLIC_CHARS.contains(b) {
                        RuleParseState::OpStarted(OpStartedState {
                            key,
                            op: b.to_string(),
                        })
                    } else if b == ';' {
                        return Ok(Some(Self {
                            conditions,
                            line_number,
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
                    RuleParseState::OpStarted(OpStartedState {
                        key,
                        op: c.to_string(),
                    })
                }

                (RuleParseState::KeyFinished(key), ';') => {
                    return Ok(Some(Self {
                        conditions,
                        line_number,
                        action: (key, String::default())
                            .try_into()
                            .context("Parsing action")?,
                    }))
                }

                (RuleParseState::OpStarted(mut s), c) => {
                    if c.is_ascii_whitespace() {
                        let OpStartedState { key, op } = s;
                        RuleParseState::OpFinished(OpFinishedState {
                            key,
                            op: op
                                .parse()
                                .with_context(|| format!("Parsing operator: {op}"))?,
                        })
                    } else {
                        s.op.push(c.as_char());
                        RuleParseState::OpStarted(s)
                    }
                }

                (RuleParseState::OpFinished(OpFinishedState { key, op }), c)
                    if !c.is_ascii_whitespace() =>
                {
                    if c != '"' {
                        bail!("Expecting \" but got {c:?}");
                    }

                    RuleParseState::ValueStarted(ValueStartedState {
                        key,
                        op,
                        value: Default::default(),
                        escaping: false,
                    })
                }

                (RuleParseState::ValueStarted(mut s), c) if s.escaping => {
                    s.value.push(c);
                    s.escaping = false;
                    RuleParseState::ValueStarted(s)
                }

                (RuleParseState::ValueStarted(mut s), c) => match c {
                    '\\' => {
                        s.escaping = true;
                        RuleParseState::ValueStarted(s)
                    }

                    '"' => {
                        let ValueStartedState { key, value, .. } = s;
                        match s.op {
                            AssignOrTest::Assign => RuleParseState::ActionEnd(
                                Action::try_from((key, value)).context("parsing action")?,
                            ),

                            AssignOrTest::TestOp(op) => {
                                let op: Op = (op, value).try_into().context("parsing op")?;
                                conditions.push(Condition { key, op });
                                RuleParseState::CondValueEnded
                            }
                        }
                    }

                    _ => {
                        s.value.push(c);
                        RuleParseState::ValueStarted(s)
                    }
                },

                (RuleParseState::ActionEnd(action), c) if !c.is_ascii_whitespace() => {
                    if c == ';' {
                        return Ok(Some(Self {
                            conditions,
                            action,
                            line_number,
                        }));
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
enum TableParseState {
    Start,
    CommentStarted,
    NameStarted(String),
    NameEnded(String),
}

impl Table {
    pub fn parse(
        reader: &mut ColRowCounter<impl Iterator<Item = char>>,
    ) -> anyhow::Result<Option<Self>> {
        let mut state = TableParseState::Start;

        while let Some(c) = reader.next() {
            match (&mut state, c) {
                (TableParseState::Start, '#') => {
                    state = TableParseState::CommentStarted;
                }

                (TableParseState::Start, c) if !c.is_whitespace() => {
                    state = TableParseState::NameStarted(c.to_string());
                }

                (TableParseState::CommentStarted, '\n') => {
                    state = TableParseState::Start;
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
                    while let Some(rule) = Rule::parse(reader).context("parsing rule")? {
                        rules.push(rule);
                    }

                    return Ok(Some(Table {
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

impl FromStr for Program {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tables = Vec::new();
        let mut counter = ColRowCounter::new(s.chars());
        while let Some(table) = Table::parse(&mut counter).context("Parsing table")? {
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

            # The main table
            main {
                # Rule number 1
                a == "1", b == "v", proxy = "1" ;
                c != "2", de in "s", f !in "bc", reject ;
            }

            t1 {
                # Empty table
            }

            t2 {
                k == "v", jump = "t1";
                # Rule
                r ~= ".*", proxy = "1";
            }

        "#;

        let Program { tables } = src.parse().expect("To parse table");

        assert_eq!(
            tables,
            vec![
                Table {
                    name: "main".to_string(),
                    rules: vec![
                        Rule {
                            line_number: 5,
                            conditions: vec![
                                Condition {
                                    key: "a".to_string(),
                                    op: Op::Equals("1".to_string())
                                },
                                Condition {
                                    key: "b".to_string(),
                                    op: Op::Equals("v".to_string())
                                },
                            ],
                            action: Action::Proxy("1".to_string())
                        },
                        Rule {
                            line_number: 6,
                            conditions: vec![
                                Condition {
                                    key: "c".to_string(),
                                    op: Op::NotEquals("2".to_string())
                                },
                                Condition {
                                    key: "de".to_string(),
                                    op: Op::In("s".to_string())
                                },
                                Condition {
                                    key: "f".to_string(),
                                    op: Op::NotIn("bc".to_string())
                                }
                            ],
                            action: Action::Reject
                        },
                    ]
                },
                Table {
                    name: "t1".to_string(),
                    rules: vec![]
                },
                Table {
                    name: "t2".to_string(),
                    rules: vec![
                        Rule {
                            line_number: 14,
                            conditions: vec![Condition {
                                key: "k".to_string(),
                                op: Op::Equals("v".to_string())
                            },],
                            action: Action::Jump("t1".to_string())
                        },
                        Rule {
                            line_number: 16,
                            conditions: vec![Condition {
                                key: "r".to_string(),
                                op: Op::RegexMatches(".*".parse().expect("to parse regex"))
                            },],
                            action: Action::Proxy("1".to_string())
                        },
                    ]
                }
            ]
        );
    }
}
