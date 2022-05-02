use std::{
    future::ready,
    net::SocketAddr,
    time::{Duration, Instant},
};

use anyhow::{bail, Context};
use async_trait::async_trait;
use bytes::Bytes;
use futures::{AsyncRead, AsyncWrite, SinkExt, StreamExt};

use crate::{
    fetch::connect_http_stream,
    io::{bind_udp, connect_tcp, AsRawFdExt, AsyncStreamCounter, UdpSocketExt},
    proxy::protocol::ProxyRequest,
    socks5::{
        Address, ClientConnRequest, ClientGreeting, Command, ConnStatusCode, UdpPacket, UdpRepr,
        AUTH_NO_PASSWORD,
    },
};

use super::{AsyncStream, BoxedSink, BoxedStream, Protocol, Stats};

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
        auths: &[AUTH_NO_PASSWORD],
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
    fn supports(&self, req: &ProxyRequest<'_>) -> bool {
        match (req, self.supports_udp) {
            (ProxyRequest::TCP { .. } | ProxyRequest::HTTP { .. }, _) => true,
            (ProxyRequest::UDP { .. }, v) => v,
            _ => false,
        }
    }

    async fn new_stream_conn(
        &self,
        req: &ProxyRequest<'_>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)> {
        let dst = match req {
            ProxyRequest::TCP { dst } | ProxyRequest::HTTP { dst, .. } => dst,
            _ => bail!("Unsupported request {req:?}"),
        };

        let start = Instant::now();

        let mut stream = connect_tcp(&self.address)
            .await
            .with_context(|| format!("Connecting to Socks5://{}", self.address))?;
        if let Some(m) = fwmark {
            stream.set_sock_mark(m)?;
        }

        let _ = request_socks5(
            &mut stream,
            &ClientConnRequest {
                cmd: Command::CONNECT_TCP,
                address: dst.clone(),
            },
        )
        .await
        .with_context(|| format!("Requesting SOCKS5 at: {}", self.address))?;

        match req {
            ProxyRequest::TCP { .. } => Ok((
                Box::new(AsyncStreamCounter::new(
                    stream,
                    stats.rx.clone(),
                    stats.tx.clone(),
                )),
                start.elapsed(),
            )),
            ProxyRequest::HTTP { dst, https, req } => {
                let mut http_stream = connect_http_stream(*https, dst, stream)
                    .await
                    .with_context(|| format!("Connecting to HTTP dst = {dst}, https = {https}"))?;
                req.to_async_writer(&mut http_stream)
                    .await
                    .with_context(|| format!("Writing http headers to {dst}"))?;
                Ok((
                    Box::new(AsyncStreamCounter::new(
                        http_stream,
                        stats.rx.clone(),
                        stats.tx.clone(),
                    )),
                    start.elapsed(),
                ))
            }
            _ => bail!("Invalid request {req:?}"),
        }
    }

    async fn new_dgram_conn(
        &self,
        req: &ProxyRequest<'_>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        match req {
            ProxyRequest::UDP {
                initial_dst,
                initial_data,
            } => {
                let mut socks_stream = connect_tcp(&self.address)
                    .await
                    .with_context(|| format!("Connecting to Socks5://{}", self.address))?;

                if let Some(m) = fwmark {
                    socks_stream.set_sock_mark(m)?;
                }

                let bounded = request_socks5(
                    &mut socks_stream,
                    &ClientConnRequest {
                        cmd: Command::BIND_UDP,
                        address: initial_dst.clone(),
                    },
                )
                .await
                .with_context(|| format!("Requesting SOCKS5 at: {}", self.address))?;

                let client = bind_udp(matches!(bounded, Address::IP(SocketAddr::V4(_)))).await?;

                if let Some(m) = fwmark {
                    client.set_sock_mark(m)?;
                }

                let tx = stats.tx.clone();
                let rx = stats.rx.clone();

                let relay_addr = bounded.resolve_first().await?;
                log::debug!("Sending to initial data to relay UDP server at {relay_addr}");
                let initial_data = UdpRepr {
                    addr: initial_dst,
                    payload: initial_data.as_ref(),
                    frag_no: 0,
                }
                .to_packet()?
                .into_inner();

                tx.inc(initial_data.len());
                client
                    .send_to(initial_data.as_ref(), relay_addr)
                    .await
                    .context("Sending initial data")?;

                let (sink, stream) = client.to_sink_stream().to_connected(relay_addr);

                Ok((
                    Box::pin(sink.with(move |(data, dst): (Bytes, Address<'static>)| {
                        let buf = UdpRepr {
                            addr: &dst,
                            frag_no: 0,
                            payload: data,
                        }
                        .to_packet()
                        .map(UdpPacket::into_inner);
                        if let Ok(b) = &buf {
                            tx.inc(b.len());
                        }
                        ready(buf)
                    })),
                    Box::pin(stream.filter_map(move |pkt| {
                        rx.inc(pkt.len());
                        ready(match UdpPacket::new_checked(pkt) {
                            Ok(p) => Some((p.payload_bytes(), p.addr().into_owned())),
                            Err(e) => {
                                log::warn!("Error parsing Sock5 UDP Packet: {e:?}");
                                None
                            }
                        })
                    })),
                ))
            }
            _ => bail!("Unsupported request: {req:?}"),
        }
    }
}
