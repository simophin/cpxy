use anyhow::{anyhow, Context};
use bytes::{Buf, BufMut};
use futures_lite::future::race;
use futures_lite::io::{copy, split};
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use serde::{de::DeserializeOwned, Serialize};
use smol::spawn;
use std::cmp::min;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

async fn copy_with_stats(
    mut r: impl AsyncRead + Unpin + Send + Sync,
    mut w: impl AsyncWrite + Unpin + Send + Sync,
    stat: &AtomicUsize,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 8192];
    loop {
        match r.read(buf.as_mut_slice()).await? {
            0 => return Ok(()),
            v => {
                stat.fetch_add(v, Ordering::Relaxed);
                w.write_all(&buf.as_slice()[..v])
                    .await
                    .context("Writing to")?;
            }
        }
    }
}

pub async fn copy_duplex(
    d1: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    d2: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    d1d2_count: Option<Arc<AtomicUsize>>,
    d2d1_count: Option<Arc<AtomicUsize>>,
) -> anyhow::Result<()> {
    let (d1r, d1w) = split(d1);
    let (d2r, d2w) = split(d2);
    let task1 = spawn(async move {
        if let Some(count) = d1d2_count {
            let _ = copy_with_stats(d1r, d2w, count.as_ref()).await?;
        } else {
            let _ = copy(d1r, d2w).await?;
        }
        anyhow::Result::<()>::Ok(())
    });
    let task2 = spawn(async move {
        if let Some(count) = d2d1_count {
            let _ = copy_with_stats(d2r, d1w, count.as_ref()).await?;
        } else {
            let _ = copy(d2r, d1w).await?;
        }
        anyhow::Result::<()>::Ok(())
    });

    let _ = race(task1, task2).await;
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

impl RWBuffer<Vec<u8>> {
    pub fn with_capacity(size: usize) -> Self {
        Self {
            buf: vec![0; size],
            read_cursor: 0,
            write_cursor: 0,
        }
    }

    pub fn new(buf: Vec<u8>) -> Self {
        Self {
            buf,
            read_cursor: 0,
            write_cursor: 0,
        }
    }
}

impl<T: AsRef<[u8]>> RWBuffer<T> {
    pub fn capacity(&self) -> usize {
        self.buf.as_ref().len()
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

    pub fn clear(&mut self) {
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
            buf.put_slice(&RWBuffer::read_buf(self)[..len]);
            self.advance_read(len);
        }
        return Ok(len);
    }
}

pub fn write_bincode_lengthed(mut buf: &mut Vec<u8>, o: &impl Serialize) -> anyhow::Result<()> {
    let prev_len = buf.len();
    buf.put_u16(0);
    serde_json::to_writer(&mut buf, o)?;
    let written_len = buf.len() - prev_len - 2;
    if written_len > u16::MAX as usize {
        return Err(anyhow!("Object is too big: {written_len} > {}", u16::MAX));
    }

    (&mut buf.as_mut_slice()[prev_len..]).put_u16(written_len as u16);
    Ok(())
}

pub async fn write_bincode_lengthed_async(
    w: &mut (impl AsyncWrite + Unpin),
    o: &impl Serialize,
) -> anyhow::Result<()> {
    let mut data = Vec::new();
    write_bincode_lengthed(&mut data, o)?;
    w.write_all(data.as_slice()).await?;
    Ok(())
}

pub async fn read_bincode_lengthed_async<T: DeserializeOwned>(
    r: &mut (impl AsyncRead + Unpin),
) -> anyhow::Result<T> {
    let mut buf = Vec::with_capacity(512);
    buf.resize(2, 0);
    r.read_exact(buf.as_mut_slice()).await?;
    let len = buf.as_slice().get_u16() as usize;
    buf.resize(len, 0);
    r.read_exact(buf.as_mut_slice()).await?;
    match serde_json::from_slice(buf.as_slice()) {
        Ok(v) => Ok(v),
        Err(e) => {
            log::error!("Error decoding json: {e}");
            Err(e.into())
        }
    }
}

pub trait JsonSerializable {
    fn to_json(&self) -> Vec<u8>;
}

impl<T: Serialize> JsonSerializable for T {
    fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Encode json")
    }
}

pub async fn write_http_response(
    w: &mut (impl AsyncWrite + Unpin + Send + Sync),
    code: u16,
    status_msg: Option<&str>,
    content_type: Option<&str>,
    body: &[u8],
) -> anyhow::Result<()> {
    w.write_all(
        format!(
            "HTTP/1.1 {code}{}\r\n\
    Content-Type: {}\r\n\
    Content-Length: {}\r\n\
    Access-Control-Allow-Headers: *\r\n\
    Access-Control-Allow-Origin: *\r\n\
    Access-Control-Allow-Methods: *\r\n\
    \r\n",
            status_msg.map(|m| format!(" {m}")).unwrap_or(String::new()),
            content_type.unwrap_or("application/octet-stream"),
            body.len()
        )
        .as_bytes(),
    )
    .await?;
    if !body.is_empty() {
        w.write_all(body).await?;
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_lengthed_encoding() {
        smol::block_on(async move {
            let data = "hello, world";
            let mut buf = Vec::<u8>::new();
            write_bincode_lengthed(&mut buf, &data).unwrap();

            let expected: String = read_bincode_lengthed_async(&mut buf.as_slice())
                .await
                .unwrap();
            assert_eq!(expected, data);

            buf.clear();
            write_bincode_lengthed_async(&mut buf, &data).await.unwrap();
            let expected: String = read_bincode_lengthed_async(&mut buf.as_slice())
                .await
                .unwrap();
            assert_eq!(expected, data);
        });
    }
}
