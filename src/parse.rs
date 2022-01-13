use std::fmt::{Debug, Display, Formatter};
use std::io::ErrorKind::UnexpectedEof;

use bytes::BufMut;
use futures::{AsyncRead, AsyncReadExt};

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
//
// pub async fn parse_async<'a: 'b, 'b, T: Parsable<'a> + 'a>(
//     r: &'b mut (impl AsyncRead + Unpin + ?Sized),
//     buf: &'a mut [u8],
// ) -> anyhow::Result<(usize, T)> {
//     let mut total_bytes_read = 0;
//     loop {
//         let bytes_read = r.read(&mut buf[total_bytes_read..]).await?;
//         if bytes_read == 0 {
//             return Err(std::io::Error::from(UnexpectedEof).into());
//         }
//
//         total_bytes_read += bytes_read;
//         let mut buf = &buf[..total_bytes_read];
//         match T::parse(&mut buf)? {
//             Some(v) => return Ok((0, v)),
//             None => continue,
//         }
//     }
// }
