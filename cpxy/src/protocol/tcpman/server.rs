use super::super::{ProtocolAcceptedState, ProtocolAcceptor, ProxyRequest};
use super::crypto;
use super::params::ConnectionParameters;
use crate::cipher::chacha20::ChaCha20;
use crate::cipher::stream::{CipherState, CipherStream};
use crate::http::utils::WithHeaders;
use crate::io::connect_tcp;
use crate::protocol::ProtocolReply;
use crate::tls::TlsStream;
use crate::ws;
use anyhow::{bail, Context};
use async_shutdown::Shutdown;
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use orion::aead::SecretKey;
use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{copy_bidirectional, AsyncBufRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

pub struct TcpmanAcceptor(SecretKey);

#[async_trait]
impl ProtocolAcceptor for TcpmanAcceptor {
    type AcceptedState = TcpmanAcceptedState;

    async fn accept(
        &self,
        stream: TcpStream,
    ) -> anyhow::Result<(Self::AcceptedState, ProxyRequest)> {
        let stream = BufReader::new(stream);

        let (recv_cipher_state, mut send_cipher_state, req, acceptor) =
            parse_tcpman_request(&self.0, stream).await?;

        Ok((
            TcpmanAcceptedState {
                recv_cipher_state,
                send_cipher_state,
            },
            ProxyRequest {
                dst: Default::default(),
                initial_data: None,
            },
        ))
    }
}

pub struct TcpmanAcceptedState {
    send_cipher_state: CipherState<ChaCha20>,
    recv_cipher_state: CipherState<ChaCha20>,
}

#[async_trait]
impl ProtocolAcceptedState for TcpmanAcceptedState {
    type ServerStream = CipherStream<BufReader<TcpStream>, ChaCha20, ChaCha20>;

    async fn reply(self, reply: ProtocolReply) -> anyhow::Result<Self::ServerStream> {
        let (mut upstream, initial_reply) = match execute_proxy_request(req).await {
            Ok(v) => v,
            Err(e) => {
                let _ = acceptor
                    .respond(false, Some(|_buf: &mut BytesMut| {}))
                    .await;
                return Err(e);
            }
        };

        let initial_data = if let Some(data) = initial_reply {
            Some(crypto::encrypt_initial_data(&mut send_cipher_state, data))
        } else {
            None
        };
        todo!()
    }
}

async fn handle_tcpman_connection(key: &SecretKey, conn: TcpStream) -> anyhow::Result<()> {
    let conn = acceptor
        .respond(
            true,
            Some(move |buf: &mut BytesMut| {
                if let Some(data) = initial_data {
                    write!(buf, "ETag: {data}\r\n").unwrap();
                }
            }),
        )
        .await
        .context("Responding ws")?;

    let mut conn = CipherStream::new(conn, send_cipher_state, recv_cipher_state, None);

    copy_bidirectional(&mut conn, &mut upstream)
        .await
        .context("copying data")?;
    Ok(())
}

async fn execute_proxy_request(
    req: ProxyRequest,
) -> anyhow::Result<(TlsStream<TcpStream>, Option<BytesMut>)> {
    log::info!("executing {req:?}");

    let upstream = connect_tcp(&req.dst)
        .await
        .context("connecting to upstream")?;

    let mut upstream = TlsStream::connect_plain(upstream)
        .await
        .context("connecting to TLS")?;

    if let Some(data) = req.initial_data {
        upstream
            .write_all(&data)
            .await
            .context("sending initial data")?;
    }

    // Read initial data and wait for up to 200ms
    let mut initial_data = BytesMut::zeroed(4096);
    let initial_data =
        match timeout(Duration::from_millis(20), upstream.read(&mut initial_data)).await {
            Ok(Ok(n)) if n > 0 => {
                initial_data.truncate(n);
                Some(initial_data)
            }

            Ok(Ok(_)) | Err(_) => None,
            Ok(Err(e)) => bail!("error reading initial data: {e}"),
        };

    return Ok((upstream, initial_data));
}

async fn parse_tcpman_request<S>(
    key: &SecretKey,
    conn: S,
) -> anyhow::Result<(
    CipherState<ChaCha20>,
    CipherState<ChaCha20>,
    ProxyRequest,
    ws::server::Acceptor<S>,
)>
where
    S: AsyncBufRead + Unpin,
{
    let (acceptor, (recv, send, req)) = ws::server::Acceptor::accept(conn, |req| {
        // Extract connection parameters from path
        let ConnectionParameters {
            upload_cipher,
            download_cipher,
            dst,
        } = ConnectionParameters::decrypt_from_path(req.path.context("missing path")?, key)
            .context("decrypting connection parameters")?;

        let mut recv_cipher_state: CipherState<_> = upload_cipher.into();
        let send_cipher_state: CipherState<_> = download_cipher.into();

        let initial_data = if let Some(data) = req.get_header("If-None-Match") {
            Some(Bytes::from(
                crypto::decrypt_initial_data(&mut recv_cipher_state, data)
                    .context("decrypting initial data")?,
            ))
        } else {
            None
        };

        Ok((
            recv_cipher_state,
            send_cipher_state,
            ProxyRequest { dst, initial_data },
        ))
    })
    .await?;

    Ok((recv, send, req, acceptor))
}
