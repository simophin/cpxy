use crate::cipher::suite::StreamCipherExt;
use cipher::{StreamCipher, StreamCipherError};

struct NoOp;

impl StreamCipher for NoOp {
    fn try_apply_keystream_inout(
        &mut self,
        buf: cipher::inout::InOutBuf<'_, '_, u8>,
    ) -> Result<(), StreamCipherError> {
        Ok(())
    }
}

impl StreamCipherExt for NoOp {
    fn will_modify_data(&self) -> bool {
        false
    }

    fn rewind(&mut self, _: usize) {}
}

pub fn new_no_op() -> impl StreamCipherExt + Send + Sync {
    NoOp
}
