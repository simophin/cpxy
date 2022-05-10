use super::super::{Protocol, Stats};
use super::proto;
use crate::io::{bind_udp, AsRawFdExt, UdpSocketExt};
use crate::protocol::{BoxedSink, BoxedStream, TrafficType};
use crate::socks5::Address;
use anyhow::{anyhow, Context};
use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use std::future::ready;
use std::net::SocketAddr;
use uuid::Uuid;

use crate::rt::{mpsc::channel, spawn};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct UdpMan {
    pub addr: Address<'static>,
}

#[async_trait]
impl Protocol for UdpMan {
    fn supports(&self, req: TrafficType) -> bool {
        req == TrafficType::Datagram
    }

    async fn new_datagram(
        &self,
        dst: &Address<'_>,
        initial_data: Bytes,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        let (tx, rx) = (stats.tx.clone(), stats.rx.clone());
        let upstream = bind_udp(matches!(self.addr, Address::IP(SocketAddr::V4(_)))).await?;

        if let Some(m) = fwmark {
            upstream.set_sock_mark(m)?;
        }
        let upstream_addr = self.addr.resolve_first().await?;

        // Send connect message
        let uuid = Uuid::new_v4();
        let initial_dst = dst.resolve_first().await?;
        let connect_msg = proto::Message::Connect {
            uuid: uuid.as_ref().into(),
            initial_data: initial_data.into(),
            dst: initial_dst,
            initial_data_nonce: None,
        }
        .to_bytes()?;
        tx.inc(connect_msg.len());
        upstream
            .send_to(&connect_msg, upstream_addr)
            .await
            .with_context(|| format!("Sending connect message to upstream {}", self.addr))?;

        let (outgoing_tx, outgoing_rx) = channel::<(Bytes, Address<'static>)>(2);
        let (incoming_tx, incoming_rx) = channel::<anyhow::Result<(Bytes, Address<'static>)>>(2);

        let (upstream_sink, upstream_stream) =
            upstream.to_sink_stream().to_connected(upstream_addr);

        let (msg_sink, msg_stream) = proto::new_message_sink_stream(
            upstream_sink.with(move |b: Bytes| {
                tx.inc(b.len());
                ready(Ok(b))
            }),
            upstream_stream.inspect_ok(move |data| rx.inc(data.len())),
        );

        let mut outgoing_rx = Some(outgoing_rx);
        let mut msg_sink = Some(msg_sink);

        spawn(
            msg_stream
                .filter_map(move |m| {
                    let m = match m {
                        Ok(v) => v,
                        Err(e) => return ready(Some(Err(e))),
                    };
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
                                                    enc_nonce: None,
                                                    payload: data.into(),
                                                })
                                            })
                                            .forward(msg_sink),
                                    )
                                    .detach();
                                    initial_reply.map(|(data, addr)| Ok((data, addr.into())))
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
                .map(|m| anyhow::Result::Ok(m))
                .forward(incoming_tx),
        )
        .detach();

        Ok((
            Box::pin(outgoing_tx.sink_map_err(anyhow::Error::from)),
            Box::pin(incoming_rx),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::test::test_protocol_udp,
        rt::{block_on, spawn},
        test::create_udp_socket,
    };

    #[test]
    fn udpman_works() {
        let _ = env_logger::try_init();
        block_on(async move {
            let (server_socket, server_addr) = create_udp_socket().await;
            let _task = spawn(super::super::server::serve_socket(server_socket));

            let protocol = UdpMan {
                addr: server_addr.into(),
            };

            test_protocol_udp(&protocol).await;
        });
    }
}
