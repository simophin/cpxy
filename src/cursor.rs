#[derive(Debug, Clone)]
pub struct Cursor<const N: usize> {
    buf: [u8; N],
    position: usize,
}

impl<const N: usize> Default for Cursor<N> {
    fn default() -> Self {
        Cursor {
            buf: [0; N],
            position: 0,
        }
    }
}

impl<const N: usize> Cursor<N> {
    pub fn reset(&mut self) {
        self.position = 0;
    }

    pub fn move_position(&mut self, add: usize) {
        self.position += add;
        assert!(self.position <= self.buf.len());
    }

    pub fn remaining(&self) -> &[u8] {
        return &self.buf[self.position..];
    }

    pub fn remaining_mut(&mut self) -> &mut [u8] {
        return &mut self.buf[self.position..];
    }

    pub fn remaining_len(&self) -> usize {
        self.capacity() - self.position
    }

    pub fn used(&self) -> &[u8] {
        return &self.buf[..self.position];
    }

    pub fn move_used_to_front(&mut self, offset_in_used: usize) {
        assert!(offset_in_used <= self.position);
        if offset_in_used == 0 {
            return;
        }

        if offset_in_used == self.position {
            self.reset();
            return;
        }

        (&mut self.buf[..self.position]).copy_within(offset_in_used..self.position, 0);
        self.position = self.position - offset_in_used;
    }

    pub const fn capacity(&self) -> usize {
        N
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;

    #[test]
    fn cursor_works() {
        let mut cursor: Cursor<200> = Default::default();
        let data = b"hello, world";
        cursor.remaining_mut().write_all(data);
        cursor.move_position(data.len());
        assert_eq!(cursor.remaining().len(), cursor.capacity() - data.len());
        assert_eq!(cursor.used(), &data[..]);

        cursor.move_used_to_front(1);
        assert_eq!(cursor.used(), &data[1..]);
    }
}
