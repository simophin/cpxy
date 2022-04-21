use std::io::{Read, Write};

use bytes::BufMut;

pub struct RWBuffer {
    buf: Vec<u8>,
    read_cursor: usize,
    write_cursor: usize,
}

impl RWBuffer {
    pub fn new(buf: Vec<u8>) -> Self {
        Self {
            buf,
            read_cursor: 0,
            write_cursor: 0,
        }
    }

    pub fn new_vec_uninitialised(len: usize) -> Self {
        Self::new(crate::utils::new_vec_uninitialised(len))
    }

    fn grow(&mut self) {
        let old_len = self.buf.len();
        let new_len = old_len * 15 / 10;
        if new_len > old_len {
            log::debug!("Growing buffer from {old_len} to {new_len}");
            self.buf.resize(new_len, 0);
        }
    }

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

    pub fn read_buf(&self) -> &[u8] {
        &self.buf[self.read_cursor..self.write_cursor]
    }

    pub fn advance_write(&mut self, cnt: usize) {
        self.write_cursor += cnt;
        assert!(self.write_cursor <= self.buf.len());
    }

    pub fn remaining_write(&self) -> usize {
        self.buf.len() - self.write_cursor
    }

    pub fn should_compact(&self) -> bool {
        self.remaining_write() < self.buf.len() / 4
    }

    pub fn write_buf(&mut self) -> &mut [u8] {
        if self.remaining_write() == 0 {
            self.grow()
        }
        &mut self.buf[self.write_cursor..]
    }

    pub fn compact(&mut self) {
        if self.read_cursor < self.write_cursor {
            if self.read_cursor > 0 {
                self.buf.copy_within(self.read_cursor..self.write_cursor, 0);
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

impl Write for RWBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.remaining_write() < buf.len() {
            self.compact();
        }

        let len = self.remaining_write().min(buf.len());
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

impl Read for RWBuffer {
    fn read(&mut self, mut buf: &mut [u8]) -> std::io::Result<usize> {
        let len = self.remaining_read().min(buf.len());
        if len > 0 {
            buf.put_slice(&RWBuffer::read_buf(self)[..len]);
            self.advance_read(len);
        }
        return Ok(len);
    }
}
