use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use smallvec::smallvec;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::io::time_future;
use crate::protocol::{BoxProtocolReporter, ProxyRequest};
use crate::tls::TlsStream;
use crate::{
    io::{connect_tcp_marked, CounterStream},
    socks5::{
        Address, ClientConnRequest, ClientGreeting, Command, ConnStatusCode, AUTH_NO_PASSWORD,
    },
};

use super::Protocol;

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
    type Stream = CounterStream<TlsStream<TcpStream>>;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        reporter: &BoxProtocolReporter,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::Stream> {
        let (mut upstream, delay) = time_future(connect_tcp_marked(&self.address, fwmark))
            .await
            .context("Connecting to SOCKS sever")?;

        reporter.report_delay(delay);

        let _ = request_socks5(
            &mut upstream,
            &ClientConnRequest {
                cmd: Command::CONNECT_TCP,
                address: req.dst.clone(),
            },
        )
        .await
        .context("Requesting SOCKS5 proxy")?;

        let upstream = TlsStream::connect_plain(upstream)
            .await
            .context("Connecting to upstream")?;

        let mut upstream = CounterStream::new(upstream, reporter.clone());
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
