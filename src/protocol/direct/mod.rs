use super::{Protocol, Stats, TrafficType};
use crate::io::{
    bind_udp, connect_tcp, send_to_addr, AsRawFdExt, AsyncStreamCounter, UdpSocketExt,
};
use crate::protocol::{AsyncStream, BoxedSink, BoxedStream};
use crate::socks5::Address;
use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{AsyncWriteExt, TryStreamExt};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Direct;

#[async_trait]
impl Protocol for Direct {
    fn supports(&self, _: TrafficType) -> bool {
        true
    }

    async fn new_stream(
        &self,
        dst: &Address<'_>,
        initial_data: Option<&[u8]>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Box<dyn AsyncStream>> {
        let mut stream = connect_tcp(dst).await?;
        if let Some(fwmark) = fwmark {
            stream.set_sock_mark(fwmark)?;
        }

        match initial_data {
            Some(b) if b.len() > 0 => {
                stream.write_all(b).await.context("Writing initial data")?;
            }
            _ => {}
        };

        Ok(Box::new(AsyncStreamCounter::new(
            stream,
            stats.rx.clone(),
            stats.tx.clone(),
        )))
    }

    async fn new_datagram(
        &self,
        dst: &Address<'_>,
        initial_data: Bytes,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        let socket = bind_udp(!matches!(dst, Address::IP(SocketAddr::V4(_))))
            .await
            .context("Binding UDP socket")?;

        if let Some(m) = fwmark {
            socket.set_sock_mark(m)?;
        }

        send_to_addr(&socket, initial_data.as_ref(), dst)
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
            Box::pin(stream.map_ok(move |(data, addr)| {
                rx.inc(data.len());
                (data, addr.into())
            })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test;
    use super::*;
    use smol::block_on;

    #[test]
    fn test_direct_works() {
        let direct = Direct {};
        block_on(test::test_protocol_tcp(&direct));
        block_on(test::test_protocol_udp(&direct));
        block_on(test::test_protocol_http(&direct));
    }
}
