
use anyhow::{bail, Context};
use bytes::Buf;
use super::op::*;

pub struct Condition {
    pub key: String,
    pub value: String,
    pub op: Op,
}

pub struct Action {
    pub what: String,
    pub how: String,
}

enum ParseState {
    Start,
    CondKeyStarted(String),
    CondKeyFinished(String),
    OpStarted {
        cond_key: String,
        op: String,
    },
    OpFinished {
        cond_key: String,
        op: Op,
    },
    CondValueStarted {
        cond_key: String,
        op: Op,

        value: String,
        escaping: bool,
    },
    CondValueFinished,
}

pub struct RawRule {
    pub conditions: Vec<Condition>,
    pub action: Action,
}


impl RawRule {
    pub fn parse(reader: &mut impl Buf) -> anyhow::Result<Self> {
        let mut state = ParseState::Start;
        let mut conditions = Vec::new();

        let mut action = None;

        while reader.has_remaining() {
            match (&mut state, reader.get_u8()) {
                (ParseState::Start, c) if !c.is_ascii_whitespace() => {
                        state = ParseState::CondKeyStarted(c.to_string());
                },
    
                (ParseState::CondKeyStarted(cond), b) => {
                    if b.is_ascii_whitespace() {
                        state = ParseState::CondKeyFinished(std::mem::take(cond));
                    } else if FIRST_OP_BYTES.contains(&b) {
                        state = ParseState::OpStarted {
                            cond_key: std::mem::take(cond),
                            op: b.to_string(),
                        };
                    } else {
                        cond.push(b as char);
                    }
                },
    
                (ParseState::CondKeyFinished(cond), c) if FIRST_OP_BYTES.contains(&c) => {
                    state = ParseState::OpStarted {
                        cond_key: std::mem::take(cond),
                        op: c.to_string(),
                    };
                },

                (ParseState::OpStarted { cond_key: cond, op }, c) => {
                    if c.is_ascii_whitespace() {
                        state = ParseState::OpFinished {
                            cond_key: std::mem::take(cond),
                            op: op.parse().with_context(|| format!("Parsing operator for cond key: {cond}"))?,
                        };
                    } else {
                        op.push(c as char);
                    }
                },

                (ParseState::OpFinished { cond_key, op }, c) if !c.is_ascii_whitespace() => {
                    let start_quote = if c == b'"' || c == b'\'' {
                        Some(c)
                    } else {
                        None
                    };

                    state = ParseState::CondValueStarted {
                        start_quote,
                        cond_key: std::mem::take(cond_key),
                        op: *op,
                        value: if start_quote.is_some() {
                            Default::default()
                        } else {
                            c.to_string()
                        },
                        escaping: false,
                    };
                },

                (ParseState::CondValueStarted { cond_key, op, start_quote, value, escaping }, c) => {
                    if *escaping {
                        value.push(c as char);
                        *escaping = false;
                        continue;
                    }

                    match (*start_quote, c) {
                        (None, _) if c.is_ascii_whitespace() => {
                            state = ParseState::CondValueFinished;
                        },
                    };
                },

                (ParseState::CondValueEscaping(_), b'')=> todo!(),
                (ParseState::CondValueFinished, b'')=> todo!(),
            }
        }
        todo!()
    }
}
