mod crypto;
mod params;

use crate::cipher::chacha20::ChaCha20;
use crate::cipher::stream::{CipherConfig, CipherStream};
use crate::io::{connect_tcp_marked, CounterStream};
use crate::protocol::tcpman::params::ConnectionParameters;
use crate::protocol::{ProxyRequest, Stats};
use crate::socks5::Address;
use crate::tls::TlsStream;
use anyhow::Context;
use async_trait::async_trait;
use bytes::BytesMut;
use once_cell::sync::OnceCell;
use orion::{aead, pwhash};
use serde::{Deserialize, Serialize};
use tokio::io::BufReader;
use tokio::net::TcpStream;

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Tcpman {
    pub address: Address<'static>,
    pub tls: bool,
    pub password: String,

    #[serde(skip)]
    key: OnceCell<aead::SecretKey>,
}

type TcpmanStream =
    TlsStream<CipherStream<BufReader<CounterStream<TcpStream>>, ChaCha20, ChaCha20>>;

#[async_trait]
impl super::Protocol for Tcpman {
    type Stream = TcpmanStream;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::Stream> {
        let upstream = CounterStream::new(
            connect_tcp_marked(&self.address, fwmark)
                .await
                .context("connecting to upstream")?,
            stats.rx.clone(),
            stats.tx.clone(),
        );

        // Connect to TLS
        let stream = if self.tls {
            TlsStream::connect_tls(self.address.get_host().as_ref(), upstream).await
        } else {
            TlsStream::connect_plain(upstream).await
        }
        .context("connecting to TLS")?;

        // Determine connection parameters
        let params = ConnectionParameters {
            upload_cipher: CipherConfig::None,
            download_cipher: CipherConfig::None,
        };

        // Construct a Websocket request
        let mut req = BytesMut::new();
        // write!(req, "GET {} HTTP/1.1\r\n", ).unwrap();

        todo!()
    }
}

impl Tcpman {
    fn key(&self) -> &aead::SecretKey {
        use orion::pwhash;
        self.key
            .get_or_try_init(|| {
                let hashed = pwhash::hash_password(
                    &pwhash::Password::from_slice(self.password.as_bytes())
                        .context("invalid password")?,
                    10,
                    2,
                )
                .context("hashing password")?;
                aead::SecretKey::from_slice(hashed.unprotected_as_bytes()).context("invalid key")
            })
            .expect("failed to derive key")
    }
}

impl Clone for Tcpman {
    fn clone(&self) -> Self {
        Self {
            address: self.address.clone(),
            tls: self.tls,
            password: self.password.clone(),
            key: Default::default(),
        }
    }
}
