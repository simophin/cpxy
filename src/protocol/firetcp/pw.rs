use blake2::{
    digest::{Update, VariableOutput},
    Blake2bVar,
};
use serde::{Deserialize, Serialize};

const HASH_CONTEXT_KEY: &'static [u8] = b"key";
const HASH_CONTEXT_NONCE: &'static [u8] = b"nonce";

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordedKey {
    pw: String,
    init_key: Vec<u8>,
    init_nonce: Vec<u8>,
}

impl PasswordedKey {
    pub fn new(pw: impl Into<String>) -> Self {
        let pw: String = pw.into();
        Self {
            init_key: Self::derive_key(pw.as_bytes(), HASH_CONTEXT_KEY, KEY_LEN),
            init_nonce: Self::derive_key(pw.as_bytes(), HASH_CONTEXT_NONCE, NONCE_LEN),
            pw,
        }
    }

    fn derive_key(master: &[u8], subkey_id: &[u8], len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        let mut hasher = Blake2bVar::new(len).unwrap();
        hasher.update(master);
        hasher.update(subkey_id);
        hasher.finalize_variable(&mut out).unwrap();
        out
    }

    pub fn init_key(&self) -> &[u8] {
        &self.init_key
    }

    pub fn init_nonce(&self) -> &[u8] {
        &self.init_nonce
    }
}

impl Serialize for PasswordedKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.pw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PasswordedKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::new(<String as Deserialize>::deserialize(
            deserializer,
        )?))
    }
}

impl From<String> for PasswordedKey {
    fn from(k: String) -> Self {
        Self::new(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_works() {
        let p = PasswordedKey::new("12345");
        assert_eq!(KEY_LEN, p.init_key().len());
        assert_eq!(NONCE_LEN, p.init_nonce().len());

        let cloned_p = p.clone();
        assert_eq!(p.init_key(), cloned_p.init_key());
        assert_eq!(p.init_nonce(), cloned_p.init_nonce());
    }
}
