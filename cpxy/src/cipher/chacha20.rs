use ::chacha20 as crypto;
use cipher::{KeyIvInit, StreamCipher as CryptoStreamCipher};

use super::StreamCipher;

pub struct ChaCha20(crypto::ChaCha20);

impl StreamCipher for ChaCha20 {
    type Key = [u8; 32];
    type Iv = [u8; 12];

    fn new(key: &Self::Key, iv: &Self::Iv) -> Self {
        Self(crypto::ChaCha20::new_from_slices(key.as_ref(), iv.as_ref()).unwrap())
    }

    fn apply_in_place(&mut self, data: &mut [u8]) {
        self.0.apply_keystream(data)
    }
}
