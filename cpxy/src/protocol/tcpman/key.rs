use anyhow::Context;
use orion::{aead, kdf};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

#[derive(SerializeDisplay, DeserializeFromStr, Eq, PartialEq)]
pub struct SecretKey {
    password: String,
    key: aead::SecretKey,
}

impl Debug for SecretKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretKey(***)")
    }
}

impl Clone for SecretKey {
    fn clone(&self) -> Self {
        Self {
            password: self.password.clone(),
            key: aead::SecretKey::from_slice(self.key.unprotected_as_bytes()).unwrap(),
        }
    }
}

impl Display for SecretKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.password, f)
    }
}

impl FromStr for SecretKey {
    type Err = anyhow::Error;

    fn from_str(password: &str) -> Result<Self, Self::Err> {
        let key = kdf::derive_key(
            &kdf::Password::from_slice(password.as_bytes()).context("creating password")?,
            &kdf::Salt::from_slice(b"tcpman_protocol_password").context("creating salt")?,
            3,
            1 << 16,
            32,
        )
        .context("deriving key")?;

        let key = aead::SecretKey::from_slice(key.unprotected_as_bytes()).context("invalid key")?;

        Ok(Self {
            password: password.to_string(),
            key,
        })
    }
}

impl AsRef<aead::SecretKey> for SecretKey {
    fn as_ref(&self) -> &aead::SecretKey {
        &self.key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_works() {
        let key1: SecretKey = "password".parse().expect("To parse password");
        let key2: SecretKey = "password".parse().expect("To parse password");

        assert_eq!(key1, key2);
    }
}
