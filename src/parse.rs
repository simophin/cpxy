use crate::cursor::Cursor;
use futures::{AsyncRead, AsyncReadExt};
use std::fmt::Debug;

#[derive(Debug)]
pub enum ParseStatus<T: Debug> {
    Incomplete,
    Completed(T),
}

pub trait Parsable: Debug {
    fn parse(buf: &[u8]) -> anyhow::Result<ParseStatus<(usize, Self)>>
    where
        Self: Sized;
}

pub async fn parse_cursor<T: Parsable, const N: usize>(
    cursor: &mut Cursor<N>,
    rx: &mut (impl AsyncRead + Unpin + ?Sized),
) -> anyhow::Result<T> {
    loop {
        let bytes_read = rx.read(cursor.remaining_mut()).await?;
        if bytes_read == 0 {
            return Err(anyhow::anyhow!("EOF"));
        }
        cursor.move_position(bytes_read);
        match T::parse(cursor.used())? {
            ParseStatus::Incomplete if cursor.remaining_len() == 0 => {
                return Err(anyhow::anyhow!("Invalid format"));
            }
            ParseStatus::Incomplete => continue,
            ParseStatus::Completed((offset, r)) => {
                cursor.move_used_to_front(offset);
                return Ok(r);
            }
        }
    }
}
