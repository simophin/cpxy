use std::num::NonZeroUsize;

use crate::addr::Address;
use crate::cipher::stream::CipherConfig;
use crate::{cipher::chacha20::ChaCha20, cipher::StreamCipher, protocol::ProxyRequest};
use anyhow::{bail, Context};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use bytes::{Buf, BufMut, Bytes};
use chrono::{DateTime, Utc};
use orion::aead;
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionParameters {
    pub upload_cipher: CipherConfig<ChaCha20>,
    pub download_cipher: CipherConfig<ChaCha20>,
    pub dst: Address,
}

impl ConnectionParameters {
    pub fn create_for_request(req: &ProxyRequest) -> Self {
        match req.dst.port() {
            443 | 22 => {
                let (key, iv) = ChaCha20::rand_key_iv();
                Self {
                    upload_cipher: CipherConfig::FirstNBytes {
                        n: NonZeroUsize::new(512).unwrap(),
                        key,
                        iv,
                    },
                    download_cipher: CipherConfig::None,
                    dst: req.dst.clone(),
                }
            }

            _ => {
                let (key, iv) = ChaCha20::rand_key_iv();
                Self {
                    upload_cipher: CipherConfig::Full { key, iv },
                    download_cipher: CipherConfig::Full { key, iv },
                    dst: req.dst.clone(),
                }
            }
        }
    }

    pub fn encrypt_to_path(&self, key: &aead::SecretKey) -> anyhow::Result<String> {
        let timestamp = DateTime::<Utc>::default().timestamp();
        let mut output = rmp_serde::to_vec(self).context("serializing connection parameters")?;
        output.put_i64(timestamp);
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
                if i < output.len() - 1 {
                    next_slash_position = rng.gen_range(i + 1..output.len());
                }
                n -= 1;
            }

            new_output.push(output[i]);
        }

        Ok(String::from_utf8(output).unwrap())
    }

    pub fn decrypt_from_path(path: &str, key: &aead::SecretKey) -> anyhow::Result<Self> {
        log::debug!("Decrypting {path}");
        // Remove all / from the path
        let path = path.replace("/", "");

        let current_timestamp = DateTime::<Utc>::default().timestamp();

        let input = B64.decode(path).context("decoding connection parameters")?;
        let output =
            Bytes::from(aead::open(&key, &input).context("decrypting connection parameters")?);

        // Split the last 8 bytes into timestamp
        let (output, mut timestamp_buffer) = output.split_at(output.len() - 8);
        let timestamp = timestamp_buffer.get_i64();

        if timestamp < current_timestamp - 360 || timestamp > current_timestamp + 360 {
            bail!("timestamp is invalid");
        }

        Ok(rmp_serde::from_slice(&output).context("deserializing connection parameters")?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_encryption_works() {
        let key = aead::SecretKey::default();

        let params = ConnectionParameters {
            upload_cipher: CipherConfig::Full {
                key: Default::default(),
                iv: Default::default(),
            },
            download_cipher: CipherConfig::Full {
                key: Default::default(),
                iv: Default::default(),
            },
            dst: "example.com:443".parse().expect("To parse address"),
        };

        let path = params.encrypt_to_path(&key).expect("To encrypt");
        let actual = ConnectionParameters::decrypt_from_path(&path, &key).expect("To decrypt");
        assert_eq!(params, actual);
    }
}
