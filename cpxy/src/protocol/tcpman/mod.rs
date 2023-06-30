mod crypto;
mod params;
pub mod server;

use crate::cipher::chacha20::ChaCha20;
use crate::cipher::stream::{CipherState, CipherStream};
use crate::http::parse_response;
use crate::http::utils::WithHeaders;
use crate::io::{connect_tcp_marked, CounterStream};
use crate::protocol::tcpman::params::ConnectionParameters;
use crate::protocol::{ProxyRequest, Stats};
use crate::socks5::Address;
use crate::tls::TlsStream;
use anyhow::{bail, Context};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use once_cell::sync::OnceCell;
use orion::aead;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Tcpman {
    address: Address<'static>,
    tls: bool,
    password: String,

    #[serde(skip)]
    key: OnceCell<aead::SecretKey>,
}

impl Tcpman {
    pub fn new(address: Address<'static>, tls: bool, password: String) -> Self {
        Self {
            address,
            tls,
            password,
            key: OnceCell::new(),
        }
    }
}

type TcpmanStream =
    CipherStream<BufReader<TlsStream<CounterStream<TcpStream>>>, ChaCha20, ChaCha20>;

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
        let mut stream = if self.tls {
            TlsStream::connect_tls(self.address.get_host().as_ref(), upstream).await
        } else {
            TlsStream::connect_plain(upstream).await
        }
        .context("connecting to TLS")?;

        // Determine connection parameters
        let params = ConnectionParameters::create_for_request(req);

        let mut send_cipher_state: CipherState<_> = params.upload_cipher.clone().into();
        let mut recv_cipher_state: CipherState<_> = params.download_cipher.clone().into();

        // Encrypt initial data
        let encrypted_initial_data = req
            .initial_data
            .as_ref()
            .map(|input| crypto::encrypt_initial_data(&mut send_cipher_state, input));

        let path = params
            .encrypt_to_path(self.key())
            .context("encrypting connection params")?;

        // Construct a Websocket request
        let mut req = BytesMut::new();
        write!(req, "GET {path} HTTP/1.1\r\n")?;
        write!(req, "Host: {}\r\n", self.address.get_host())?;
        write!(req, "Connection: Upgrade\r\n")?;
        write!(req, "Upgrade: websocket\r\n")?;
        if let Some(data) = encrypted_initial_data {
            write!(req, "If-None-Match: {data}\r\n")?;
        }
        write!(req, "\r\n")?;

        stream.write_all(&req).await.context("Writing request")?;

        let mut stream = BufReader::new(stream);

        // Wait for protocol switch
        let plaintext_initial_reply = parse_response(&mut stream, |res| {
            if res.code != Some(101) {
                bail!("Expecting protocol switch status but got: {:?}", res.code);
            }

            if let Some(data) = res.get_header("ETag") {
                Ok(Some(Bytes::from(
                    crypto::decrypt_initial_data(&mut recv_cipher_state, &data)
                        .context("decrypting initial data")?,
                )))
            } else {
                Ok(None)
            }
        })
        .await?;

        Ok(CipherStream::new(
            stream,
            send_cipher_state,
            recv_cipher_state,
            plaintext_initial_reply,
        ))
    }
}

impl Tcpman {
    fn key(&self) -> &aead::SecretKey {
        self.key
            .get_or_try_init(|| Self::derive_key(&self.password))
            .expect("failed to derive key")
    }

    pub fn derive_key(password: &str) -> anyhow::Result<aead::SecretKey> {
        use orion::pwhash;

        let hashed = pwhash::hash_password(
            &pwhash::Password::from_slice(password.as_bytes()).context("invalid password")?,
            10,
            2,
        )
        .context("hashing password")?;
        aead::SecretKey::from_slice(hashed.unprotected_as_bytes()).context("invalid key")
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
