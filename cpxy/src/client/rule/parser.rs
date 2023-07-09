use std::{io::BufRead, num::NonZeroUsize};

use anyhow::Context;
use byteorder::ReadBytesExt;
use non_empty_string::NonEmptyString;

pub enum Op {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    RegexMatches,
}

pub struct Condition {
    pub key: String,
    pub value: String,
    pub op: Op,
}

pub struct Action {
    pub what: String,
    pub how: String,
}

pub enum ParseState {
    Start,
    CondStarted(NonEmptyString),
    CondFinished,
    OpStarted(u8),
    OpFinished,
    ValueStarted(String),
    ValueEscaping(String),
    ValueFinished,
}

pub struct ParseContext<B> {
    pub line_number: NonZeroUsize,
    pub reader: B,
    pub state: ParseState,
}

pub struct RawRule {
    pub line_number: NonZeroUsize,
    pub conditions: Vec<Condition>,
    pub action: Action,
}

const FIRST_OP_BYTES: &[u8] = b"=~!";

impl RawRule {
    pub fn parse(ctx: &mut ParseContext<impl BufRead>) -> anyhow::Result<Self> {
        match (&mut ctx.state, ctx.reader.read_u8()) {
            (ParseState::Start, Ok(c)) if !c.is_ascii_whitespace() => {
                ctx.state = ParseState::CondStarted(NonEmptyString::new(c.to_string()).unwrap());
            },

            (ParseState::CondStarted(cond), Ok(b)) if !b.is_ascii_whitespace() => {
                cond.push(b as char);
            },

            (ParseState::CondStarted(cond), Ok(b)) if b.is_ascii_whitespace() => {
                ctx.state = ParseState::CondFinished;
            },
            
            (ParseState::CondStarted(_), Err(e)) => {

            }

            (ParseState::CondFinished, b'')=> todo!(),
            (ParseState::OpStarted(_), b'')=> todo!(),
            (ParseState::OpFinished, b'')=> todo!(),
            (ParseState::ValueStarted(_), b'')=> todo!(),
            (ParseState::ValueEscaping(_), b'')=> todo!(),
            (ParseState::ValueFinished, b'')=> todo!(),
        }
        todo!()
    }
}
