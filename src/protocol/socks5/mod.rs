use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use smallvec::smallvec;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::protocol::ProxyRequest;
use crate::tls::TlsStream;
use crate::{
    io::{connect_tcp_marked, AsyncStreamCounter},
    socks5::{
        Address, ClientConnRequest, ClientGreeting, Command, ConnStatusCode, AUTH_NO_PASSWORD,
    },
};

use super::{Protocol, Stats};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Socks5 {
    pub address: Address<'static>,
    pub supports_udp: bool,
}

async fn request_socks5(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    req: &ClientConnRequest<'_>,
) -> anyhow::Result<Address<'static>> {
    // Send greeting
    ClientGreeting {
        auths: smallvec![AUTH_NO_PASSWORD],
    }
    .to_async_writer(stream)
    .await
    .context("Sending greeting message")?;

    // Expect greeting respond
    let auth = ClientGreeting::read_response(stream)
        .await
        .context("Receiving greeting response")?;

    if auth != AUTH_NO_PASSWORD {
        bail!("Expecting NO_PASSWORD AUTH");
    }

    // Send request
    req.to_async_writer(stream)
        .await
        .context("Sending conn req")?;

    // Expect returns
    let (code, addr) = ClientConnRequest::parse_response(stream)
        .await
        .context("Receiving conn response")?;

    if code != ConnStatusCode::GRANTED {
        bail!("Invalid socks5 status code: {code:?}");
    }

    Ok(addr)
}

#[async_trait]
impl Protocol for Socks5 {
    type Stream = AsyncStreamCounter<TlsStream<TcpStream>>;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::Stream> {
        let mut upstream = connect_tcp_marked(&self.address, fwmark)
            .await
            .context("Connecting to SOCKS sever")?;

        let _ = request_socks5(
            &mut upstream,
            &ClientConnRequest {
                cmd: Command::CONNECT_TCP,
                address: req.dst.clone(),
            },
        )
        .await
        .context("Requesting SOCKS5 proxy")?;

        let upstream = if req.tls {
            TlsStream::connect_tls(self.address.get_host().as_ref(), upstream).await
        } else {
            TlsStream::connect_plain(upstream).await
        }
        .context("Connecting to upstream")?;

        let mut upstream = AsyncStreamCounter::new(upstream, stats.rx.clone(), stats.tx.clone());
        match &req.initial_data {
            Some(b) if b.len() > 0 => upstream
                .write_all(b)
                .await
                .context("Writing initial data")?,
            _ => {}
        }

        Ok(upstream)
    }
}
