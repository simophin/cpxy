use super::Protocol;
use crate::fetch::send_http;
use crate::io::{bind_udp, connect_tcp, send_to_addr, UdpSocketExt};
use crate::protocol::{AsyncStream, BoxedSink, BoxedStream};
use crate::proxy::protocol::ProxyRequest;
use crate::socks5::Address;
use anyhow::{anyhow, bail, Context};
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
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)> {
        let start = Instant::now();
        match req {
            ProxyRequest::TCP { dst } => Ok((Box::new(connect_tcp(dst).await?), start.elapsed())),
            ProxyRequest::HTTP { https, dst, req } => Ok((
                Box::new(send_http(*https, dst, req).await?),
                start.elapsed(),
            )),
            _ => bail!("Unsupported stream connection for {req:?}"),
        }
    }

    async fn new_dgram_conn(
        &self,
        req: &ProxyRequest<'_>,
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
                Ok((
                    Box::pin(
                        sink.with(|(data, addr): (Bytes, Address<'static>)| async move {
                            Ok((
                                data,
                                addr.resolve_first()
                                    .await?
                                    .ok_or_else(|| anyhow!("Unable to resolve {addr}"))?,
                            ))
                        }),
                    ),
                    Box::pin(stream.map(|(data, addr)| (data, addr.into()))),
                ))
            }
            _ => bail!("Unsupported datagram connection for {req:?}"),
        }
    }
}
