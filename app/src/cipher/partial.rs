use super::suite::StreamCipherExt;
use cipher::{StreamCipher, StreamCipherError};
use std::cmp::{max, min};
use std::num::NonZeroUsize;

struct PartialStreamCipher<T: StreamCipherExt + Send + Sync> {
    inner: T,
    n: isize,
}

impl<T: StreamCipherExt + Send + Sync> StreamCipherExt for PartialStreamCipher<T> {
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

pub fn new_partial_stream_cipher(
    n: NonZeroUsize,
    inner: impl StreamCipherExt + Send + Sync,
) -> impl StreamCipherExt + Send + Sync {
    PartialStreamCipher {
        inner,
        n: n.get().try_into().unwrap(),
    }
}

impl<T: StreamCipherExt + Send + Sync> StreamCipher for PartialStreamCipher<T> {
    fn try_apply_keystream_inout(
        &mut self,
        mut buf: cipher::inout::InOutBuf<'_, '_, u8>,
    ) -> Result<(), StreamCipherError> {
        if self.n > 0 {
            let len = min(self.n as usize, buf.len());
            self.inner.try_apply_keystream(&mut buf.get_out()[..len])?;
            self.n = self.n.checked_sub(buf.len().try_into().unwrap()).unwrap();
        }

        Ok(())
    }
}
