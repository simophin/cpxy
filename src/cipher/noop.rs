use crate::cipher::suite::BoxedStreamCipher;
use cipher::errors::LoopError;
use cipher::StreamCipher;

struct NoOp;

impl StreamCipher for NoOp {
    fn try_apply_keystream(&mut self, _: &mut [u8]) -> Result<(), LoopError> {
        Ok(())
    }
}

pub fn new_no_op() -> BoxedStreamCipher {
    Box::new(NoOp)
}
