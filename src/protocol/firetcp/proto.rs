use anyhow::{bail, Context};
use async_trait::async_trait;
use chacha20::ChaCha20;
use cipher::KeyIvInit;
use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};

use super::super::{AsyncStream, Protocol, Stats, TrafficType};
use crate::{
    io::{connect_tcp_marked, union, AsyncStreamCounter},
    socks5::Address,
};

pub struct FireTcp {
    pub server: Address<'static>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CipherOption {
    key: [u8; 32],
    nonce: [u8; 12],
}

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    addr: Address<'static>,
    initial_data_len: usize,
    est_cipher: Option<CipherOption>,
}

const INITIAL_CIPHER_LEN: usize = 512;
const INITIAL_KEY: &'static [u8] = b"P2$3M$5RRsTY49oo#3xQwT3vE6MVDpck";
const INITIAL_NONCE: &'static [u8] = b"oU151Hq8J@!q";

#[async_trait]
impl Protocol for FireTcp {
    fn supports(&self, traffic_type: TrafficType) -> bool {
        traffic_type == TrafficType::Stream
    }

    async fn new_stream(
        &self,
        dst: &Address<'_>,
        initial_data: Option<&[u8]>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Box<dyn AsyncStream>> {
        let (r, w) = AsyncStreamCounter::new(
            connect_tcp_marked(&self.server, fwmark)
                .await
                .context("Connecting to firetcp server")?,
            stats.rx.clone(),
            stats.tx.clone(),
        )
        .split();

        let mut w = super::cipher::CipherWrite::<_, _, ChaCha20>::new(
            w,
            INITIAL_CIPHER_LEN,
            ChaCha20::new_from_slices(INITIAL_KEY, INITIAL_NONCE).unwrap(),
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

        let req_buf = serde_json::to_vec(&Request {
            addr: dst.clone().into_owned(),
            initial_data_len: initial_data.map(|v| v.len()).unwrap_or_default(),
            est_cipher,
        })
        .context("Unable to serialize request to json")?;

        let req_buf_len: u16 = req_buf.len().try_into().context("Request too big")?;
        w.write_all(&req_buf_len.to_be_bytes())
            .await
            .context("Writing request len")?;
        w.write_all(&req_buf).await.context("Sending request")?;
        if let Some(b) = initial_data {
            w.write_all(b).await.context("Sending initial data")?;
        }

        Ok(Box::new(union(r, w)))
    }
}
