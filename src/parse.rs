use std::fmt::{Debug, Display, Formatter};
use std::future::Future;
use std::io::BufRead;
use std::io::ErrorKind::UnexpectedEof;

use bytes::BufMut;
use futures::{AsyncBufRead, AsyncRead, AsyncReadExt};

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

pub type ParseResult<'a, T> = Result<Option<(&'a [u8], T)>, ParseError>;

pub trait Parsable<'a>: Sized + 'a {
    fn parse(buf: &'a [u8]) -> ParseResult<'a, Self>;
}

pub trait Writable {
    fn write_len(&self) -> usize;
    fn write(&self, buf: &mut impl BufMut) -> bool;
}
