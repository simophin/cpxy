use anyhow::anyhow;
use bytes::BufMut;
use futures_lite::io::{copy, split};
use futures_lite::{AsyncRead, AsyncWrite};
use futures_util::future::join;
use serde_derive::{Deserialize, Serialize};
use smol::spawn;
use std::cmp::min;
use std::io::{Read, Write};

pub async fn copy_duplex(
    d1: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    d2: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let (d1r, d1w) = split(d1);
    let (d2r, d2w) = split(d2);
    let task1 = spawn(async move { copy(d1r, d2w).await });
    let task2 = spawn(async move { copy(d2r, d1w).await });

    let _ = join(task1, task2).await;
    Ok(())
}

pub struct RWBuffer<T = Vec<u8>> {
    buf: T,
    read_cursor: usize,
    write_cursor: usize,
}

impl Default for RWBuffer<Vec<u8>> {
    fn default() -> Self {
        RWBuffer::with_capacity(8192)
    }
}

impl From<Vec<u8>> for RWBuffer<Vec<u8>> {
    fn from(buf: Vec<u8>) -> Self {
        Self {
            read_cursor: 0,
            write_cursor: buf.len(),
            buf,
        }
    }
}

impl RWBuffer<Vec<u8>> {
    pub fn with_capacity(size: usize) -> Self {
        Self {
            buf: vec![0; size],
            read_cursor: 0,
            write_cursor: 0,
        }
    }
}

impl<T> RWBuffer<T> {
    pub fn remaining_read(&self) -> usize {
        return self.write_cursor - self.read_cursor;
    }

    pub fn advance_read(&mut self, cnt: usize) {
        self.read_cursor += cnt;
        assert!(self.read_cursor <= self.write_cursor);
        if self.read_cursor == self.write_cursor {
            self.read_cursor = 0;
            self.write_cursor = 0;
        }
    }

    pub fn consume_read(&mut self) {
        self.read_cursor = 0;
        self.write_cursor = 0;
    }
}

impl<T: AsMut<[u8]> + AsRef<[u8]>> RWBuffer<T> {
    pub fn read_buf(&self) -> &[u8] {
        &self.buf.as_ref()[self.read_cursor..self.write_cursor]
    }

    pub fn advance_write(&mut self, cnt: usize) {
        self.write_cursor += cnt;
        assert!(self.write_cursor <= self.buf.as_ref().len());
    }

    pub fn remaining_write(&self) -> usize {
        self.buf.as_ref().len() - self.write_cursor
    }

    pub fn write_buf(&mut self) -> &mut [u8] {
        &mut self.buf.as_mut()[self.write_cursor..]
    }

    pub fn read_buf_mut(&mut self) -> &mut [u8] {
        &mut self.buf.as_mut()[self.read_cursor..self.write_cursor]
    }

    pub fn should_compact(&self) -> bool {
        self.remaining_write() < self.buf.as_ref().len() / 4
    }

    pub fn compact(&mut self) {
        if self.read_cursor < self.write_cursor {
            if self.read_cursor > 0 {
                self.buf
                    .as_mut()
                    .copy_within(self.read_cursor..self.write_cursor, 0);
                self.write_cursor -= self.read_cursor;
                self.read_cursor = 0;
            }
        } else if self.read_cursor == self.write_cursor {
            self.read_cursor = 0;
            self.write_cursor = 0;
        } else {
            unreachable!()
        }
    }
}

impl<T: AsMut<[u8]> + AsRef<[u8]>> Write for RWBuffer<T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.remaining_write() < buf.len() {
            self.compact();
        }

        let len = min(buf.len(), self.remaining_write());
        if len > 0 {
            self.write_buf().put_slice(&buf[..len]);
            self.advance_write(len);
        }
        return Ok(len);
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<T: AsMut<[u8]> + AsRef<[u8]>> Read for RWBuffer<T> {
    fn read(&mut self, mut buf: &mut [u8]) -> std::io::Result<usize> {
        let len = min(buf.len(), self.remaining_read());
        if len > 0 {
            buf.put_slice(&self.read_buf()[..len]);
            self.advance_read(len);
        }
        return Ok(len);
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl TryFrom<httparse::Request<'_, '_>> for HttpRequest {
    type Error = anyhow::Error;

    fn try_from(req: httparse::Request<'_, '_>) -> Result<Self, Self::Error> {
        Ok(HttpRequest {
            method: req
                .method
                .map(|x| x.to_string())
                .ok_or_else(|| anyhow!("Missing method"))?,
            path: req
                .path
                .map(|x| x.to_string())
                .ok_or_else(|| anyhow!("Missing path"))?,
            headers: req
                .headers
                .iter()
                .map(|x| {
                    (
                        x.name.to_string(),
                        String::from_utf8_lossy(x.value).to_string(),
                    )
                })
                .collect(),
        })
    }
}
