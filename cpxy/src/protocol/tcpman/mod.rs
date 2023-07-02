mod crypto;
mod params;
pub mod server;

use crate::addr::Address;
use crate::cipher::chacha20::ChaCha20;
use crate::cipher::stream::{CipherState, CipherStream};
use crate::http::utils::WithHeaders;
use crate::io::{connect_tcp_marked, time_future, CounterStream};
use crate::protocol::tcpman::params::ConnectionParameters;
use crate::protocol::{BoxProtocolReporter, ProxyRequest};
use crate::tls::TlsStream;
use crate::ws;
use anyhow::Context;
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use once_cell::sync::OnceCell;
use orion::aead;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use tokio::io::BufReader;
use tokio::net::TcpStream;

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Tcpman {
    address: Address,
    tls: bool,
    password: String,

    #[serde(skip)]
    key: OnceCell<aead::SecretKey>,
}

impl Tcpman {
    pub fn new(address: Address, tls: bool, password: impl ToString) -> Self {
        Self {
            address,
            tls,
            password: password.to_string(),
            key: OnceCell::new(),
        }
    }
}

type TcpmanStream =
    CipherStream<BufReader<TlsStream<CounterStream<TcpStream>>>, ChaCha20, ChaCha20>;

#[async_trait]
impl super::Protocol for Tcpman {
    type ClientStream = TcpmanStream;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        reporter: &BoxProtocolReporter,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::ClientStream> {
        let (stream, tcp_delay) = time_future(connect_tcp_marked(&self.address, fwmark))
            .await
            .context("connecting to upstream")?;

        let stream = CounterStream::new(stream, reporter.clone());

        // Connect to TLS
        let (stream, tls_delay) = if self.tls {
            time_future(TlsStream::connect_tls(self.address.host(), stream)).await?
        } else {
            time_future(TlsStream::connect_plain(stream)).await?
        };

        reporter.report_delay(tcp_delay + tls_delay);

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

        let mut stream = BufReader::new(stream);

        let plaintext_initial_reply = ws::client::connect(
            &mut stream,
            path,
            self.address.host(),
            Some(move |req: &mut BytesMut| {
                if let Some(data) = encrypted_initial_data {
                    write!(req, "If-None-Match: {data}\r\n").unwrap();
                }
            }),
            |res| {
                if let Some(data) = res.get_header("ETag") {
                    Ok(Some(Bytes::from(
                        crypto::decrypt_initial_data(&mut recv_cipher_state, &data)
                            .context("decrypting initial data")?,
                    )))
                } else {
                    Ok(None)
                }
            },
        )
        .await
        .context("Connecting to ws")?;

        Ok(CipherStream::new(
            stream,
            send_cipher_state,
            recv_cipher_state,
            plaintext_initial_reply,
        ))
    }

    #[cfg(test)]
    async fn test_servers() -> Vec<(Self, tokio::task::JoinHandle<anyhow::Result<()>>)> {
        use tokio::net::TcpListener;
        let server_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("To bind a tcp listener");
        let server = Tcpman::new(
            server_listener
                .local_addr()
                .expect("To have a local address")
                .into(),
            false,
            "123456",
        );

        let key = aead::SecretKey::from_slice(server.key().unprotected_as_bytes()).unwrap();

        vec![(
            server,
            tokio::spawn(server::run_tcpman_server(
                Default::default(),
                key,
                server_listener,
            )),
        )]
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
