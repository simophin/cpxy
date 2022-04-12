use std::time::Duration;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use smol::spawn;
use smol_timeout::TimeoutExt;

use crate::{
    buf::RWBuffer,
    fetch::send_http,
    handshake::{HandshakeRequest, Handshaker},
    io::TcpStream,
    utils::{copy_duplex, read_bincode_lengthed_async, write_bincode_lengthed_async},
    ws::negotiate_websocket,
};

use super::{server::ConnectionType, ConnectionId};

#[derive(Serialize, Deserialize, Debug)]
pub enum ClientCommand {
    NewConnection { id: ConnectionId },
}

async fn serve_new_conn(id: String, url: String) -> anyhow::Result<()> {
    let mut conn_stream = negotiate_websocket(&url, vec![])
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
        HandshakeRequest::TCP { dst } => {
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
        HandshakeRequest::HTTP { dst, https, req } => {
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
    let mut control_stream = negotiate_websocket(&url, vec![])
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
