use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn copy_io(
    mut r: impl AsyncRead + Unpin,
    mut w: impl AsyncWrite + Unpin,
) -> anyhow::Result<()> {
    let mut buf = vec![0; 8192];
    loop {
        match r.read(buf.as_mut_slice()).await? {
            0 => return Ok(()),
            v => {
                w.write_all(&buf.as_slice()[..v]).await?;
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

    pub fn read_buf(&self) -> &[u8] {
        &self.buf.as_slice()[self.read_cursor..self.write_cursor]
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
