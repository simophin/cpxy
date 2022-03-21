use std::{borrow::Cow, time::Duration};

use anyhow::{bail, Context};
use futures_lite::{AsyncRead, AsyncWrite};
use serde::{Deserialize, Serialize};
use smol::spawn;
use smol_timeout::TimeoutExt;

use crate::{
    buf::RWBuffer,
    fetch::send_http,
    handshake::Handshaker,
    http::{parse_response, HttpCommon, HttpRequest},
    io::TcpStream,
    proxy::protocol::ProxyRequest,
    utils::{copy_duplex, read_bincode_lengthed_async, write_bincode_lengthed_async},
};

use super::{server::ConnectionType, ConnectionId};

#[derive(Serialize, Deserialize, Debug)]
pub enum ClientCommand {
    NewConnection { id: ConnectionId },
}

async fn establish_conn(
    url: &str,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync> {
    let u = crate::url::HttpUrl::try_from(url).context("Parsing server URL")?;
    let host = u.address.get_host();

    let req = HttpRequest {
        common: HttpCommon {
            headers: vec![
                (Cow::Borrowed("Connection"), "Upgrade".into()),
                (Cow::Borrowed("Upgrade"), "Websocket".into()),
                (Cow::Borrowed("Sec-WebSocket-Version"), "13".into()),
                (
                    Cow::Borrowed("Sec-WebSocket-Key"),
                    "dGhlIHNhbXBsZSBub25jZQ==".into(),
                ),
                (Cow::Borrowed("Host"), host.as_ref().into()),
            ],
        },
        method: Cow::Borrowed("GET"),
        path: u.path,
    };

    let http_stream = parse_response(
        send_http(u.is_https, &u.address, req)
            .await
            .context("Sending initial request")?,
        RWBuffer::new(1024, 65536),
    )
    .await
    .context("Parsing initial response")?;

    if http_stream.status_code != 101 {
        bail!("Expecting 101 response but got {}", http_stream.status_code);
    }

    Ok(http_stream)
}

async fn serve_new_conn(id: String, url: String) -> anyhow::Result<()> {
    let mut conn_stream = establish_conn(&url)
        .timeout(Duration::from_secs(5))
        .await
        .context("Timeout waiting for connection {id}")?
        .context("Establishing connection")?;

    write_bincode_lengthed_async(&mut conn_stream, &ConnectionType::Connection { id })
        .await
        .context("Write conn request")?;

    let (handshaker, req) =
        Handshaker::start(&mut conn_stream, None, &mut RWBuffer::new(512, 2048)).await?;

    match req {
        ProxyRequest::TCP { dst } => {
            let upstream = match TcpStream::connect(&dst).await {
                Ok(v) => {
                    handshaker
                        .respond_ok(&mut conn_stream, v.local_addr().ok())
                        .await?;
                    v
                }
                Err(e) => {
                    log::error!("Error connecting to tcp://{dst}: {e:?}");
                    handshaker.respond_err(&mut conn_stream).await?;
                    return Err(e.into());
                }
            };

            copy_duplex(upstream, conn_stream, None, None).await
        }
        ProxyRequest::HTTP { dst, https, req } => {
            let upstream = match send_http(https, &dst, req).await {
                Ok(v) => {
                    handshaker.respond_ok(&mut conn_stream, None).await?;
                    v
                }
                Err(e) => {
                    log::error!("Error connecting to http(s)://{dst}: {e:?}");
                    handshaker.respond_err(&mut conn_stream).await?;
                    return Err(e.into());
                }
            };

            copy_duplex(upstream, conn_stream, None, None).await
        }
        v => {
            handshaker.respond_err(&mut conn_stream).await?;
            bail!("Unsupported proxy request {v:?}");
        }
    }
}

pub async fn run_client(url: String) -> anyhow::Result<()> {
    let mut control_stream = establish_conn(&url)
        .timeout(Duration::from_secs(5))
        .await
        .context("Timeout waiting for control connection")?
        .context("Establishing control connection")?;

    log::info!("Connected to controller at {url}");

    write_bincode_lengthed_async(&mut control_stream, &ConnectionType::Main)
        .await
        .context("Writing control header")?;

    loop {
        match read_bincode_lengthed_async(&mut control_stream).await? {
            ClientCommand::NewConnection { id } => {
                let url = url.clone();
                spawn(async move {
                    if let Err(e) = serve_new_conn(id, url).await {
                        log::error!("Error serving conn id: {e:?}");
                    }
                })
                .detach();
            }
        }
    }
}
