use crate::socks5::Address;
use lazy_static::lazy_static;
use orion::aead;
use orion::aead::SecretKey;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::net::SocketAddr;

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum Request<'a> {
    Init {
        initial_dst: Address<'a>,
        initial_data: Cow<'a, [u8]>,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Response<'a> {
    SingleReply {
        data: Cow<'a, [u8]>,
        src: SocketAddr,
    },
    Redirect {
        port: u16,
    },
}

fn secret_key() -> &'static SecretKey {
    lazy_static! {
        static ref KEY: SecretKey =
            SecretKey::from_slice(b"=DW5<W;FJ;nPMA`&6cCpzm7jJNp3`J4a").unwrap();
    }
    &KEY
}

pub fn encrypt_control_message(msg: &impl Serialize) -> anyhow::Result<Vec<u8>> {
    Ok(aead::seal(secret_key(), &serde_json::to_vec(msg)?)?)
}

pub fn decrypt_control_message<T: DeserializeOwned>(msg: &[u8]) -> anyhow::Result<T> {
    let msg = aead::open(secret_key(), msg)?;
    Ok(serde_json::from_slice(&msg)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encryption_works() {
        let req = Request::Init {
            initial_dst: Default::default(),
            initial_data: Cow::Borrowed(b"hello, world"),
        };

        let actual: Request =
            decrypt_control_message(&encrypt_control_message(&req).expect("To encrypt"))
                .expect("To decrypt");
        assert_eq!(req, actual)
    }
}
