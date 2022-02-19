use std::{
    collections::VecDeque,
    io::{Read, Write},
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    sync::Mutex,
};

use bytes::BufMut;
use lazy_static::lazy_static;

struct Buf(Option<Box<[u8]>>);

impl Deref for Buf {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self.0.as_ref() {
            Some(b) => b.as_ref(),
            None => b"",
        }
    }
}

impl DerefMut for Buf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self.0.as_mut() {
            Some(b) => b.as_mut(),
            None => panic!("Should not deref_mut an empty slice"),
        }
    }
}

pub struct RWBuffer {
    buf: Buf,
    read_cursor: usize,
    write_cursor: usize,
    max_len: usize,
}

const MAX_BUFFER_LEN: usize = 128;

fn new_buf(s: usize) -> Box<[u8]> {
    let reused = match buffer_cache().lock() {
        Ok(mut g) => match g.binary_search_by(|v| v.len().cmp(&s)) {
            Ok(index) => g.remove(index),
            Err(index) if g.len() > 0 && index < g.len() - 1 => g.remove(index + 1),
            _ => None,
        },
        Err(_) => None,
    };

    if let Some(reused) = reused {
        log::debug!("Reusing buffer size = {}, requesting = {}", reused.len(), s);
        return reused;
    }

    log::debug!("Creating new buffer size = {s}");
    unsafe { vec![MaybeUninit::<u8>::uninit().assume_init(); s].into_boxed_slice() }
}

fn recycle_buf(b: Box<[u8]>) {
    match buffer_cache().lock() {
        Ok(mut g) if g.len() < MAX_BUFFER_LEN => {
            let len = b.len();
            log::debug!("Recycling buffer with size = {len}");
            match g.binary_search_by(|v| v.len().cmp(&len)) {
                Ok(index) => g.insert(index, b),
                Err(index) => g.insert(index, b),
            }
        }
        _ => (),
    }
}

fn buffer_cache() -> &'static Mutex<VecDeque<Box<[u8]>>> {
    lazy_static! {
        static ref CACHE: Mutex<VecDeque<Box<[u8]>>> = Default::default();
    }

    &CACHE
}

impl Drop for RWBuffer {
    fn drop(&mut self) {
        match self.buf.0.take() {
            Some(b) => recycle_buf(b),
            None => {}
        }
    }
}

impl RWBuffer {
    pub fn new(init_capacity: usize, max_len: usize) -> Self {
        assert!(max_len >= init_capacity);
        Self {
            buf: Buf(Some(new_buf(init_capacity))),
            read_cursor: 0,
            write_cursor: 0,
            max_len,
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

    pub fn grow(&mut self) {
        let old_len = self.buf.len();
        let new_len = self.max_len.min(old_len * 15 / 10);
        if new_len > old_len {
            log::debug!("Growing buffer from {old_len} to {new_len}");
            let mut new_buf = new_buf(new_len);
            (&mut new_buf[..old_len]).copy_from_slice(self.buf.as_ref());
            match self.buf.0.take() {
                Some(old_buf) => recycle_buf(old_buf),
                _ => {}
            };
            self.buf = Buf(Some(new_buf));
        }
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
