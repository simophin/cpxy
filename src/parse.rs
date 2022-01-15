use std::fmt::{Debug, Display, Formatter};

use bytes::BufMut;

#[derive(Debug)]
pub enum ParseError {
    Unexpected {
        name: &'static str,
        expect: &'static str,
        got: Box<dyn Debug + Send + Sync>,
    },
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        <ParseError as Debug>::fmt(self, f)
    }
}

impl ParseError {
    pub fn unexpected(
        name: &'static str,
        got: impl Debug + Sync + Send + 'static,
        expect: &'static str,
    ) -> Self {
        Self::Unexpected {
            name,
            got: Box::new(got),
            expect,
        }
    }
}

impl std::error::Error for ParseError {}

pub type ParseResult<T> = Result<Option<(usize, T)>, ParseError>;

pub trait Parsable: Sized {
    fn parse(buf: &[u8]) -> ParseResult<Self>;
}

pub trait Writable {
    fn write_len(&self) -> usize;
    fn write(&self, buf: &mut impl BufMut) -> bool;
}
