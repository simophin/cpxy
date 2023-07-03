use anyhow::Context;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub mod chacha20;
pub mod stream;

pub trait StreamCipher {
    type Key: AsRef<[u8]>
        + AsMut<[u8]>
        + Default
        + Clone
        + Serialize
        + DeserializeOwned
        + Eq
        + PartialEq
        + 'static;
    type Iv: AsRef<[u8]>
        + AsMut<[u8]>
        + Default
        + Clone
        + Serialize
        + DeserializeOwned
        + Eq
        + PartialEq
        + 'static;

    fn new(key: &Self::Key, iv: &Self::Iv) -> Self;

    fn apply_in_place(&mut self, data: &mut [u8]);

    fn rand_key_iv() -> (Self::Key, Self::Iv) {
        let mut key = Self::Key::default();
        let mut iv = Self::Iv::default();
        orion::util::secure_rand_bytes(key.as_mut()).unwrap();
        orion::util::secure_rand_bytes(iv.as_mut()).unwrap();
        (key, iv)
    }

    fn rand_iv() -> Self::Iv {
        let mut iv = Self::Iv::default();
        orion::util::secure_rand_bytes(iv.as_mut()).unwrap();
        iv
    }

    fn derive_key(password: &str, context: &str) -> anyhow::Result<Self::Key> {
        use orion::kdf::*;

        let mut key = Self::Key::default();
        let gen = derive_key(
            &Password::from_slice(password.as_bytes()).context("invalid password")?,
            &Salt::from_slice(context.as_bytes()).context("invalid salt")?,
            10,
            2,
            key.as_ref().len() as u32,
        )
        .context("deriving key")?;

        key.as_mut().copy_from_slice(gen.unprotected_as_bytes());
        Ok(key)
    }
}
