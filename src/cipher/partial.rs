use super::suite::BoxedStreamCipher;
use cipher::errors::LoopError;
use cipher::StreamCipher;
use std::cmp::min;
use std::num::NonZeroUsize;

struct PartialStreamCipher {
    inner: BoxedStreamCipher,
    n: usize,
}

pub fn new_partial_stream_cipher(n: NonZeroUsize, inner: BoxedStreamCipher) -> BoxedStreamCipher {
    Box::new(PartialStreamCipher { inner, n: n.get() })
}

impl StreamCipher for PartialStreamCipher {
    fn try_apply_keystream(&mut self, data: &mut [u8]) -> Result<(), LoopError> {
        if self.n > 0 {
            let len = min(self.n, data.len());
            self.inner.try_apply_keystream(&mut data[..len])?;
            self.n -= len;
        }

        Ok(())
    }
}
