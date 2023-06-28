use crate::cipher::chacha20::ChaCha20;
use crate::cipher::stream::CipherConfig;
use anyhow::Context;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use orion::aead;
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionParameters {
    pub upload_cipher: CipherConfig<ChaCha20>,
    pub download_cipher: CipherConfig<ChaCha20>,
}

impl ConnectionParameters {
    pub fn encrypt_to_path(&self, key: &aead::SecretKey) -> anyhow::Result<String> {
        let output = rmp_serde::to_vec(self).context("serializing connection parameters")?;
        let output = aead::seal(&key, &output).context("encrypting connection parameters")?;
        let output = B64.encode(output).into_bytes();

        // Pick a number from 1 to 5
        let mut rng = rand::thread_rng();
        let mut n = rng.gen_range(1usize..=5);

        let mut new_output = Vec::with_capacity(output.len() + n);
        let mut next_slash_position = rng.gen_range(0usize..output.len());

        for i in 0..output.len() {
            if i == next_slash_position && n > 0 {
                new_output.push(b'/');
                next_slash_position = rng.gen_range(i + 1..output.len());
                n -= 1;
            }

            new_output.push(output[i]);
        }

        Ok(String::from_utf8(output).unwrap())
    }

    pub fn decrypt_from_path(path: &str, key: &aead::SecretKey) -> anyhow::Result<Self> {
        // Remove all / from the path
        let path = path.replace("/", "");

        let input = B64.decode(path).context("decoding connection parameters")?;
        let output = aead::open(&key, &input).context("decrypting connection parameters")?;
        Ok(rmp_serde::from_slice(&output).context("deserializing connection parameters")?)
    }
}
