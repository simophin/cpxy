use super::crypto;
use super::params::ConnectionParameters;
use crate::cipher::chacha20::ChaCha20;
use crate::cipher::stream::{CipherState, CipherStream};
use crate::http::parse_request;
use crate::http::utils::WithHeaders;
use crate::io::connect_tcp;
use crate::protocol::ProxyRequest;
use crate::tls::TlsStream;
use crate::ws;
use anyhow::{bail, Context};
use async_shutdown::Shutdown;
use bytes::{Bytes, BytesMut};
use orion::aead::SecretKey;
use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

pub async fn run_tcpman_server(
    shutdown: Shutdown,
    key: SecretKey,
    listener: TcpListener,
) -> anyhow::Result<()> {
    let key = Arc::new(key);
    loop {
        let (conn, addr) = listener.accept().await.context("accepting connection")?;
        log::info!("accepted connection from {addr}");

        let shutdown = shutdown.clone();
        let key = key.clone();
        tokio::spawn(async move {
            if let Some(Err(e)) = shutdown
                .wrap_cancel(handle_tcpman_connection(&key, conn))
                .await
            {
                log::error!("error handling connection: {e:?}");
            }

            log::info!("disconnected from {addr}");
        });
    }
}

async fn handle_tcpman_connection(key: &SecretKey, conn: TcpStream) -> anyhow::Result<()> {
    let mut conn = BufReader::new(conn);

    let (recv_cipher_state, mut send_cipher_state, req) =
        parse_tcpman_request(key, &mut conn).await?;
    let (mut upstream, initial_reply) = execute_proxy_request(req).await?;

    let initial_data = if let Some(data) = initial_reply {
        Some(crypto::encrypt_initial_data(&mut send_cipher_state, data))
    } else {
        None
    };

    // Response a Protocol switch response
    let mut resp = BytesMut::new();
    write!(resp, "HTTP/1.1 101 OK\r\n")?;
    if let Some(data) = initial_data {
        write!(resp, "ETag: {data}\r\n")?;
    }
    write!(resp, "\r\n")?;
    conn.write_all(&mut resp)
        .await
        .context("sending response")?;

    let mut conn = CipherStream::new(conn, send_cipher_state, recv_cipher_state, None);

    copy_bidirectional(&mut conn, &mut upstream)
        .await
        .context("copying data")?;
    Ok(())
}

async fn execute_proxy_request(
    req: ProxyRequest,
) -> anyhow::Result<(TlsStream<TcpStream>, Option<Vec<u8>>)> {
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
    let mut initial_data = vec![0u8; 4096];
    let initial_data =
        match timeout(Duration::from_millis(20), upstream.read(&mut initial_data)).await {
            Ok(Ok(n)) if n > 0 => {
                initial_data.resize(n, 0);
                Some(initial_data)
            }

            Ok(Ok(_)) | Err(_) => None,
            Ok(Err(e)) => bail!("error reading initial data: {e}"),
        };

    return Ok((upstream, initial_data));
}

async fn parse_tcpman_request(
    key: &SecretKey,
    mut conn: BufReader<TcpStream>,
) -> anyhow::Result<(CipherState<ChaCha20>, CipherState<ChaCha20>, ProxyRequest)> {
    parse_request(&mut conn, |req| {
        // Check for ws upgrade
        match (
            req.get_header_text("Connection"),
            req.get_header_text("Upgrade"),
        ) {
            (Some(c), Some(u))
                if c.eq_ignore_ascii_case("upgrade") && u.eq_ignore_ascii_case("websocket") =>
            {
                // Do nothing
            }
            _ => bail!("not a websocket upgrade request"),
        };

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
    .await
}
