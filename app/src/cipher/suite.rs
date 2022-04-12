use anyhow::anyhow;
use cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use rand::Rng;

pub trait StreamCipherExt: StreamCipher {
    fn will_modify_data(&self) -> bool;

    fn rewind(&mut self, cnt: usize);
}

impl StreamCipherExt for chacha20::ChaCha20 {
    fn will_modify_data(&self) -> bool {
        true
    }

    fn rewind(&mut self, cnt: usize) {
        self.seek(self.current_pos::<usize>() - cnt)
    }
}

pub type CipherType = u8;
pub type CipherKey = Vec<u8>;
pub type CipherIv = Vec<u8>;

pub fn create_cipher(
    cipher_type: CipherType,
    key: &[u8],
    iv: &[u8],
) -> anyhow::Result<impl StreamCipherExt + Send + Sync + 'static> {
    match cipher_type {
        1 => Ok(chacha20::ChaCha20::new_from_slices(key, iv)
            .map_err(|_| anyhow!("Invalid key/iv lengths for Chacha20 cipher"))?),
        _ => Err(anyhow!("Unknown cipher_type {cipher_type}")),
    }
}

pub fn pick_cipher() -> (
    CipherType,
    impl StreamCipherExt + Send + Sync + 'static,
    CipherKey,
    CipherIv,
) {
    let mut key;
    let mut iv;
    let cipher_type;

    key = vec![0u8; 32];
    iv = vec![0u8; 12];
    log::info!("Use Chacha20 cipher");
    cipher_type = 1;
    rand::thread_rng().fill(key.as_mut_slice());
    rand::thread_rng().fill(iv.as_mut_slice());

    let cipher = chacha20::ChaCha20::new_from_slices(key.as_slice(), iv.as_slice())
        .expect("to create chacha20 cipher");

    (cipher_type, cipher, key, iv)
}
