mod proto;
mod stream;

use super::Protocol;
use crate::io::{bind_udp, is_one_off_udp_query, send_to_addr, UdpSocketExt};
use crate::protocol::{AsyncStream, BoxedSink, BoxedStream};
use crate::proxy::protocol::ProxyRequest;
use crate::socks5::Address;
use anyhow::{anyhow, bail, Context};
use async_stream::try_stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::SinkExt;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::time::Duration;

use crate::rt::mpsc::channel;
use crate::utils::{new_vec_for_udp, VecExt};

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

                let upstream_addr = self
                    .addr
                    .resolve_first()
                    .await?
                    .ok_or_else(|| anyhow!("Unable to resolve upstream addr: {}", self.addr))?;

                let cmsg = proto::Request::Init {
                    initial_dst: initial_dst.clone(),
                    initial_data: Cow::Borrowed(initial_data.as_ref()),
                };

                let cmsg =
                    proto::encrypt_control_message(&cmsg).context("Encrypting control message")?;

                upstream
                    .send_to(&cmsg, upstream_addr)
                    .await
                    .context("Sending initial control message")?;

                let (tx, rx) = channel::<(Bytes, Address<'static>)>(2);

                let stream = {
                    // Receive control message
                    let mut buf = new_vec_for_udp();
                    let (len, from) = upstream.recv_from(&mut buf).await?;
                    buf.set_len_uninit(len);

                    match proto::decrypt_control_message(&buf).context("Decrypting response")? {
                        proto::Response::Redirect { port } => {
                            log::debug!("Upstream requested to redirect to Port = {port}");
                            let (upstream_sink, upstream_stream) =
                                upstream.to_sink_stream().to_connected(upstream_addr);
                        }
                        proto::Response::SingleReply { data, src } => todo!(),
                    }
                };

                Ok((
                    Box::pin(tx.sink_map_err(anyhow::Error::from)),
                    Box::pin(stream),
                ))
            }
            _ => bail!("UDPMan only supports UDP packet"),
        }
    }
}
