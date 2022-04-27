use super::Protocol;
use crate::fetch::send_http;
use crate::io::{bind_udp, connect_tcp, send_to_addr, UdpSocketExt};
use crate::protocol::{AsyncStream, BoxedSink, BoxedStream};
use crate::proxy::protocol::ProxyRequest;
use crate::socks5::Address;
use anyhow::{bail, Context};
use futures_util::StreamExt;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

pub struct Direct;

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
                Ok((Box::new(sink), Box::new(stream)))
            }
            _ => bail!("Unsupported datagram connection for {req:?}"),
        }
    }
}
