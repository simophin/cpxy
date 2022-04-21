use std::{pin::Pin, task::Poll};

use futures::AsyncRead;

pub struct VecStream(Vec<u8>, usize);

impl VecStream {
    pub fn new(v: Vec<u8>) -> Self {
        Self(v, 0)
    }
}

impl AsyncRead for VecStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let remaining = self.0.len() - self.1;
        let start = self.1;
        let len = remaining.min(buf.len());
        let end = start + len;
        (&mut buf[..len]).copy_from_slice(&self.0[start..end]);
        self.1 += len;
        Poll::Ready(Ok(len))
    }
}
