use crate::cipher::suite::{BoxedStreamCipher, StreamCipherExt};
use cipher::errors::LoopError;
use cipher::StreamCipher;

struct NoOp;

impl StreamCipher for NoOp {
    fn try_apply_keystream(&mut self, _: &mut [u8]) -> Result<(), LoopError> {
        Ok(())
    }
}

impl StreamCipherExt for NoOp {
    fn will_modify_data(&self) -> bool {
        false
    }

    fn rewind(&mut self, _: usize) {}
}

pub fn new_no_op() -> BoxedStreamCipher {
    Box::new(NoOp)
}
