use bytes::BufMut;
use std::cmp::min;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn copy_io(
    mut r: impl AsyncRead + Unpin,
    mut w: impl AsyncWrite + Unpin,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::with_capacity(8192);
    loop {
        match r.read(buf.write_buf()).await? {
            0 => return Ok(()),
            v => {
                buf.advance_write(v);
                w.write_all(buf.read_buf()).await?;
                buf.consume_read();
            }
        }
    }
}

pub async fn copy_io_with_buf(
    mut r: impl AsyncRead + Unpin,
    mut w: impl AsyncWrite + Unpin,
    mut buf: RWBuffer,
) -> anyhow::Result<()> {
    loop {
        match r.read(buf.write_buf()).await? {
            0 => return Ok(()),
            v => {
                buf.advance_write(v);
                w.write_all(buf.read_buf()).await?;
                buf.consume_read();
            }
        }
    }
}

pub struct RWBuffer {
    buf: Vec<u8>,
    read_cursor: usize,
    write_cursor: usize,
}

impl RWBuffer {
    pub fn with_capacity(size: usize) -> Self {
        Self {
            buf: vec![0; size],
            read_cursor: 0,
            write_cursor: 0,
        }
    }

    pub fn write_buf(&mut self) -> &mut [u8] {
        &mut self.buf.as_mut_slice()[self.write_cursor..]
    }

    pub fn remaining_write(&self) -> usize {
        self.buf.len() - self.write_cursor
    }

    pub fn read_buf(&self) -> &[u8] {
        &self.buf.as_slice()[self.read_cursor..self.write_cursor]
    }

    pub fn remaining_read(&self) -> usize {
        return self.write_cursor - self.read_cursor;
    }

    pub fn read_buf_mut(&mut self) -> &mut [u8] {
        &mut self.buf.as_mut_slice()[self.read_cursor..self.write_cursor]
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

    pub fn advance_write(&mut self, cnt: usize) {
        self.write_cursor += cnt;
        assert!(self.write_cursor <= self.buf.len());
    }

    pub fn compact(&mut self) {
        if self.read_cursor < self.write_cursor {
            self.buf.copy_within(self.read_cursor..self.write_cursor, 0);
            self.write_cursor = self.read_cursor;
            self.read_cursor = 0;
        } else if self.read_cursor == self.write_cursor {
            self.read_cursor = 0;
            self.write_cursor = 0;
        } else {
            unreachable!()
        }
    }
}

impl std::io::Write for RWBuffer {
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

impl std::io::Read for RWBuffer {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = min(buf.len(), self.remaining_read());
        if len > 0 {
            buf.put_slice(&self.read_buf()[..len]);
            self.advance_read(len);
        }
        return Ok(len);
    }
}
