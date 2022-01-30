use super::suite::BoxedStreamCipher;
use crate::cipher::suite::StreamCipherExt;
use cipher::errors::LoopError;
use cipher::StreamCipher;
use std::cmp::{max, min};
use std::num::NonZeroUsize;

struct PartialStreamCipher {
    inner: BoxedStreamCipher,
    n: isize,
}

impl StreamCipherExt for PartialStreamCipher {
    fn will_modify_data(&self) -> bool {
        self.n > 0
    }

    fn rewind(&mut self, cnt: usize) {
        let old_n = self.n;
        self.n = self.n.checked_add(cnt.try_into().unwrap()).unwrap();
        if self.n > 0 {
            self.inner.rewind((self.n - max(old_n, 0)) as usize);
        }
    }
}

pub fn new_partial_stream_cipher(n: NonZeroUsize, inner: BoxedStreamCipher) -> BoxedStreamCipher {
    Box::new(PartialStreamCipher {
        inner,
        n: n.get().try_into().unwrap(),
    })
}

impl StreamCipher for PartialStreamCipher {
    fn try_apply_keystream(&mut self, data: &mut [u8]) -> Result<(), LoopError> {
        if self.n > 0 {
            let len = min(self.n as usize, data.len());
            self.inner.try_apply_keystream(&mut data[..len])?;
            self.n = self.n.checked_sub(data.len().try_into().unwrap()).unwrap();
        }

        Ok(())
    }
}
