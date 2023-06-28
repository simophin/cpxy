use anyhow::Context;
use async_trait::async_trait;
use chacha20::ChaCha20;
use cipher::KeyIvInit;
use serde::{Deserialize, Serialize};
use tokio::io::{split, AsyncWriteExt};

use super::{
    super::{AsyncStream, Protocol, Stats},
    pw::PasswordedKey,
};
use crate::io::StreamUnion;
use crate::{
    io::{connect_tcp_marked, union, AsyncStreamCounter},
    socks5::Address,
    utils::write_bincode_lengthed_async,
};
use crate::protocol::ProxyRequest;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
pub struct FireTcp {
    address: Address<'static>,
    #[serde(default = "default_key")]
    password: PasswordedKey,
}

fn default_key() -> PasswordedKey {
    PasswordedKey::new("")
}

impl FireTcp {
    pub fn new(address: Address<'static>, password: PasswordedKey) -> Self {
        Self { address, password }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CipherOption {
    pub key: [u8; 32],
    pub nonce: [u8; 12],
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub addr: Address<'static>,
    pub initial_data_len: usize,
    pub est_cipher: Option<CipherOption>,
}

pub const INITIAL_CIPHER_LEN: usize = 512;

#[async_trait]
impl Protocol for FireTcp {
    type Stream = ();

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::Stream> {
        let (r, w) = split(AsyncStreamCounter::new(
            connect_tcp_marked(&self.address, fwmark)
                .await
                .context("Connecting to firetcp server")?,
            stats.rx.clone(),
            stats.tx.clone(),
        ));

        let mut w = super::cipher::CipherWrite::<_, _, ChaCha20>::new(
            w,
            INITIAL_CIPHER_LEN,
            ChaCha20::new_from_slices(self.password.init_key(), self.password.init_nonce())
                .unwrap(),
        );

        let est_cipher = match dst.get_port() {
            443 => None,
            _ => Some(CipherOption {
                key: rand::random(),
                nonce: rand::random(),
            }),
        };
        w.set_establish_cipher(
            est_cipher
                .as_ref()
                .map(|opt| ChaCha20::new_from_slices(&opt.key, &opt.nonce).unwrap()),
        )
        .context("Unable to set establish cipher")?;

        write_bincode_lengthed_async(
            &mut w,
            &Request {
                addr: dst.clone().into_owned(),
                initial_data_len: initial_data.map(|v| v.len()).unwrap_or_default(),
                est_cipher,
            },
        )
        .await
        .context("Unable to send request")?;

        if let Some(b) = initial_data {
            w.write_all(b).await.context("Sending initial data")?;
        }

        Ok(Box::new(union(r, w)))
    }
}
