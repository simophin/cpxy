use super::{Protocol, Stats};
use crate::fetch::connect_http;
use crate::io::{
    bind_udp, connect_tcp, send_to_addr, AsyncStreamCounter, TcpStreamExt, UdpSocketExt,
};
use crate::protocol::{AsyncStream, BoxedSink, BoxedStream};
use crate::proxy::protocol::ProxyRequest;
use crate::socks5::Address;
use anyhow::{bail, Context};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Direct;

#[async_trait]
impl Protocol for Direct {
    fn supports(&self, req: &ProxyRequest<'_>) -> bool {
        match req {
            ProxyRequest::EchoTestTcp | ProxyRequest::EchoTestUdp => false,
            _ => true,
        }
    }

    async fn new_stream_conn(
        &self,
        req: &ProxyRequest<'_>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)> {
        let start = Instant::now();
        match req {
            ProxyRequest::TCP { dst } => {
                let stream = connect_tcp(dst).await?;
                if let Some(fwmark) = fwmark {
                    stream.set_sock_mark(fwmark)?;
                }

                Ok((
                    Box::new(AsyncStreamCounter::new(
                        stream,
                        stats.rx.clone(),
                        stats.tx.clone(),
                    )),
                    start.elapsed(),
                ))
            }
            ProxyRequest::HTTP { https, dst, req } => {
                let stream = connect_http(*https, dst).await?;
                if let Some(mark) = fwmark {
                    stream.inner().set_sock_mark(mark)?;
                }
                let mut stream =
                    AsyncStreamCounter::new(stream, stats.rx.clone(), stats.tx.clone());
                req.to_async_writer(&mut stream).await?;
                Ok((Box::new(stream), start.elapsed()))
            }
            _ => bail!("Unsupported stream connection for {req:?}"),
        }
    }

    async fn new_dgram_conn(
        &self,
        req: &ProxyRequest<'_>,
        stats: &Stats,
        _: Option<u32>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        match req {
            ProxyRequest::UDP {
                initial_dst,
                initial_data,
            } => {
                let socket = bind_udp(!matches!(initial_dst, Address::IP(SocketAddr::V4(_))))
                    .await
                    .context("Binding UDP socket")?;
                send_to_addr(&socket, initial_data.as_ref(), initial_dst)
                    .await
                    .context("Sending initial data")?;

                let (sink, stream) = socket.to_sink_stream().split();
                let tx = stats.tx.clone();
                let rx = stats.rx.clone();
                Ok((
                    Box::pin(sink.with(move |(data, addr): (Bytes, Address<'static>)| {
                        tx.inc(data.len());
                        async move { Ok((data, addr.resolve_first().await?)) }
                    })),
                    Box::pin(stream.map(move |(data, addr)| {
                        rx.inc(data.len());
                        (data, addr.into())
                    })),
                ))
            }
            _ => bail!("Unsupported datagram connection for {req:?}"),
        }
    }
}
