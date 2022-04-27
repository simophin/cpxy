mod proto;
mod stream;

use super::Protocol;
use crate::io::{bind_udp, UdpSocketExt};
use crate::protocol::{AsyncStream, BoxedSink, BoxedStream};
use crate::proxy::protocol::ProxyRequest;
use crate::socks5::Address;
use anyhow::{bail, Context};
use async_trait::async_trait;
use futures_util::SinkExt;
use std::net::SocketAddr;
use std::time::Duration;

pub struct UdpMan {
    pub addr: Address<'static>,
}

#[async_trait]
impl Protocol for UdpMan {
    fn supports(&self, req: &ProxyRequest<'_>) -> bool {
        matches!(req, ProxyRequest::UDP { .. })
    }

    async fn new_stream_conn(
        &self,
        _: &ProxyRequest<'_>,
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)> {
        bail!("UDPMan only supports UDP connection")
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
                let upstream =
                    bind_udp(matches!(self.addr, Address::IP(SocketAddr::V4(_)))).await?;

                // upstream_sink.send().await.context("Send initial data");

                todo!()
            }
            _ => bail!("UDPMan only supports UDP packet"),
        }
    }
}
