use super::super::{ProtocolAcceptedState, ProtocolAcceptor, ProxyRequest};
use super::crypto;
use super::key::SecretKey;
use super::params::ConnectionParameters;
use crate::cipher::chacha20::ChaCha20;
use crate::cipher::stream::{CipherState, CipherStream};
use crate::http::utils::WithHeaders;
use crate::http::writer::HeaderWriter;
use crate::ws;
use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use hyper::header;
use tokio::io::{AsyncBufRead, BufReader};
use tokio::net::TcpStream;

#[derive(Clone)]
pub struct TcpmanAcceptor(pub SecretKey);

#[async_trait]
impl ProtocolAcceptor for TcpmanAcceptor {
    type AcceptedState = TcpmanAcceptedState;

    async fn accept(
        &self,
        stream: TcpStream,
    ) -> anyhow::Result<(Self::AcceptedState, ProxyRequest)> {
        let stream = BufReader::new(stream);

        let (recv_cipher_state, send_cipher_state, req, acceptor) =
            parse_tcpman_request(&self.0, stream).await?;

        Ok((
            TcpmanAcceptedState {
                recv_cipher_state,
                send_cipher_state,
                ws_acceptor: acceptor,
            },
            req,
        ))
    }
}

pub struct TcpmanAcceptedState {
    send_cipher_state: CipherState<ChaCha20>,
    recv_cipher_state: CipherState<ChaCha20>,
    ws_acceptor: ws::server::Acceptor<BufReader<TcpStream>>,
}

#[async_trait]
impl ProtocolAcceptedState for TcpmanAcceptedState {
    type ServerStream = CipherStream<BufReader<TcpStream>, ChaCha20, ChaCha20>;

    async fn reply_success(
        self,
        initial_data: Option<Bytes>,
    ) -> anyhow::Result<Self::ServerStream> {
        let Self {
            mut send_cipher_state,
            recv_cipher_state,
            ws_acceptor,
        } = self;

        let initial_data = if let Some(data) = initial_data {
            Some(crypto::encrypt_initial_data(&mut send_cipher_state, data))
        } else {
            None
        };

        let conn = ws_acceptor
            .respond(
                true,
                Option::<&str>::None,
                Some(move |writer: &mut HeaderWriter| {
                    if let Some(data) = initial_data {
                        writer.write_header(header::ETAG, data);
                    }
                }),
            )
            .await
            .context("Responding success")?;

        Ok(CipherStream::new(
            conn,
            send_cipher_state,
            recv_cipher_state,
            None,
        ))
    }

    async fn reply_error(
        mut self,
        error: Option<impl AsRef<str> + Send + Sync>,
    ) -> anyhow::Result<()> {
        self.ws_acceptor
            .respond(false, error, Option::<fn(&mut HeaderWriter) -> ()>::None)
            .await
            .context("Responding error")?;
        Ok(())
    }
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
        } = ConnectionParameters::decrypt_from_path(
            req.path.context("missing path")?,
            key.as_ref(),
        )
        .context("decrypting connection parameters")?;

        let mut recv_cipher_state: CipherState<_> = upload_cipher.into();
        let send_cipher_state: CipherState<_> = download_cipher.into();

        let initial_data = if let Some(data) = req.get_header(header::IF_NONE_MATCH) {
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
