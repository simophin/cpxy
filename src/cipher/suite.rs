use anyhow::anyhow;
use cipher::{NewCipher, StreamCipher};
use rand::Rng;

#[cfg(any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64"))]
cpufeatures::new!(cpuid_aes, "aes");

#[cfg(not(any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64")))]
mod cpuid_aes {
    pub const fn get() -> bool {
        false
    }
}

pub type CipherType = u8;
pub type CipherKey = Vec<u8>;
pub type CipherIv = Vec<u8>;

pub fn create_cipher(
    cipher_type: CipherType,
    key: &[u8],
    iv: &[u8],
) -> anyhow::Result<Box<dyn StreamCipher + Send + Sync>> {
    match cipher_type {
        0 => Ok(Box::new(aes::Aes128Ctr::new_from_slices(key, iv).map_err(
            |_| anyhow!("Invalid key/iv lengths for AES cipher"),
        )?)),
        1 => Ok(Box::new(
            chacha20::ChaCha20::new_from_slices(key, iv)
                .map_err(|_| anyhow!("Invalid key/iv lengths for Chacha20 cipher"))?,
        )),
        _ => Err(anyhow!("Unknown cipher_type {cipher_type}")),
    }
}

pub fn pick_cipher() -> (
    CipherType,
    Box<dyn StreamCipher + Send + Sync>,
    CipherKey,
    CipherIv,
) {
    let mut key;
    let mut iv;
    let mut cipher_type;
    let cipher: Box<dyn StreamCipher + Send + Sync> = if cpuid_aes::get() {
        key = vec![0u8; 16];
        iv = vec![0u8; 16];
        rand::thread_rng().fill(key.as_mut_slice());
        rand::thread_rng().fill(iv.as_mut_slice());
        log::info!("Use AES cipher");
        cipher_type = 0;
        Box::new(
            aes::Aes128Ctr::new_from_slices(key.as_slice(), iv.as_slice())
                .expect("To create Aes128 cipher"),
        )
    } else {
        key = vec![0u8; 32];
        iv = vec![0u8; 12];
        log::info!("Use Chacha20 cipher");
        cipher_type = 1;
        rand::thread_rng().fill(key.as_mut_slice());
        rand::thread_rng().fill(iv.as_mut_slice());
        Box::new(
            chacha20::ChaCha20::new_from_slices(key.as_slice(), iv.as_slice())
                .expect("to create chacha20 cipher"),
        )
    };

    (cipher_type, cipher, key, iv)
}
