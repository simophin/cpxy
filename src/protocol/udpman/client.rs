use super::super::{Protocol, Stats};
use super::proto;
use crate::io::{bind_udp, UdpSocketExt};
use crate::protocol::{AsyncStream, BoxedSink, BoxedStream};
use crate::proxy::protocol::ProxyRequest;
use crate::socks5::Address;
use anyhow::{anyhow, bail, Context};
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use futures_util::SinkExt;
use std::future::ready;
use std::net::SocketAddr;
use std::time::Duration;
use uuid::Uuid;

use crate::rt::{mpsc::channel, spawn};

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
        _: &Stats,
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)> {
        bail!("UDPMan only supports UDP connection")
    }

    async fn new_dgram_conn(
        &self,
        req: &ProxyRequest<'_>,
        stats: &Stats,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        let (tx, rx) = (stats.tx.clone(), stats.rx.clone());
        match req {
            ProxyRequest::UDP {
                initial_dst,
                initial_data,
            } => {
                let upstream =
                    bind_udp(matches!(self.addr, Address::IP(SocketAddr::V4(_)))).await?;
                let upstream_addr = self.addr.resolve_first().await?;

                // Send connect message
                let uuid = Uuid::new_v4();
                let initial_dst = initial_dst.resolve_first().await?;
                let connect_msg = proto::Message::Connect {
                    uuid: proto::BytesRef::Ref(uuid.as_ref()),
                    initial_data: proto::BytesRef::Ref(initial_data.as_ref()),
                    dst: initial_dst,
                }
                .to_bytes()?;
                tx.inc(connect_msg.len());
                upstream
                    .send_to(&connect_msg, upstream_addr)
                    .await
                    .with_context(|| {
                        format!("Sending connect message to upstream {}", self.addr)
                    })?;

                let (outgoing_tx, outgoing_rx) = channel::<(Bytes, Address<'static>)>(2);
                let (incoming_tx, incoming_rx) = channel::<(Bytes, Address<'static>)>(2);

                let (upstream_sink, upstream_stream) =
                    upstream.to_sink_stream().to_connected(upstream_addr);

                let (msg_sink, msg_stream) = proto::new_message_sink_stream(
                    upstream_sink.with(move |b: Bytes| {
                        tx.inc(b.len());
                        ready(Ok(b))
                    }),
                    upstream_stream.inspect(move |data| rx.inc(data.len())),
                );

                let mut outgoing_rx = Some(outgoing_rx);
                let mut msg_sink = Some(msg_sink);

                spawn(
                    msg_stream
                        .filter_map(move |m| {
                            log::debug!("Received Message: {m:?}");
                            ready(match m {
                                proto::Message::Establish {
                                    uuid: received_uuid,
                                    conn_id,
                                    initial_reply,
                                } => {
                                    if uuid.as_ref() != received_uuid.as_ref() {
                                        Some(Err(anyhow!("Received different UUID than requested")))
                                    } else {
                                        if let (Some(outgoing_rx), Some(msg_sink)) =
                                            (outgoing_rx.take(), msg_sink.take())
                                        {
                                            log::debug!("UDP conn established");
                                            spawn(
                                                outgoing_rx
                                                    .map(move |(data, _)| {
                                                        Ok(proto::Message::Data {
                                                            conn_id: Some(conn_id),
                                                            addr: None,
                                                            payload: proto::BytesRef::Bytes(data),
                                                        })
                                                    })
                                                    .forward(msg_sink),
                                            )
                                            .detach();
                                            initial_reply
                                                .map(|(data, addr)| Ok((data, addr.into())))
                                        } else {
                                            log::warn!("Received duplicated Establish message");
                                            None
                                        }
                                    }
                                }
                                proto::Message::Data { payload, addr, .. } => {
                                    Some(Ok((payload.into(), addr.unwrap_or(initial_dst).into())))
                                }
                                _ => None,
                            })
                        })
                        .forward(incoming_tx.sink_map_err(anyhow::Error::from)),
                )
                .detach();

                Ok((
                    Box::pin(outgoing_tx.sink_map_err(anyhow::Error::from)),
                    Box::pin(incoming_rx),
                ))
            }
            _ => bail!("UDPMan only supports UDP packet"),
        }
    }
}
