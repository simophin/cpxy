use std::{
    collections::VecDeque,
    io::{Read, Write},
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    sync::Mutex,
};

use bytes::BufMut;
use lazy_static::lazy_static;

pub struct Buf(Vec<u8>);

impl Buf {
    pub fn new_with_vec(u: Vec<u8>) -> Self {
        Buf(u)
    }

    pub fn new(min_capacity: usize) -> Self {
        Buf(Vec::from(new_buf(min_capacity)))
    }

    pub fn new_with_len(min_capacity: usize, len: usize) -> Self {
        let mut s = Self::new(min_capacity);
        s.set_len(len);
        s
    }

    pub fn new_for_udp() -> Self {
        Self::new_with_len(65536, 65536)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    pub fn set_len(&mut self, new_len: usize) {
        assert!(new_len <= self.capacity());
        unsafe { self.0.set_len(new_len) }
    }
}

impl Drop for Buf {
    fn drop(&mut self) {
        match std::mem::take(&mut self.0) {
            v if v.capacity() > 0 => recycle_buf(v.into_boxed_slice()),
            _ => {}
        }
    }
}

impl AsRef<[u8]> for Buf {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl Deref for Buf {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Buf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Write for Buf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
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

impl RWBuffer {
    pub fn new(init_capacity: usize, max_len: usize) -> Self {
        assert!(max_len >= init_capacity);
        Self {
            buf: Buf::new_with_len(init_capacity, init_capacity),
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
            let mut new_buf = Buf::new_with_len(new_len, new_len);
            (&mut new_buf[..old_len]).copy_from_slice(self.buf.as_ref());
            self.buf = new_buf;
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
