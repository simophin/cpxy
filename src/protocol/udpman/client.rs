use super::super::{Protocol, Stats};
use super::proto::{self, Message};
use crate::io::{bind_udp, AsRawFdExt, UdpSocketExt};
use crate::protocol::{BoxedSink, BoxedStream, TrafficType};
use crate::socks5::Address;
use crate::utils::race;
use anyhow::{anyhow, Context};
use async_trait::async_trait;
use bytes::Bytes;
use futures::channel::mpsc::channel;
use futures::{StreamExt, TryStreamExt};
use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use smol::{spawn, Task};
use smol_timeout::TimeoutExt;
use std::future::ready;
use std::net::SocketAddr;
use std::time::Duration;
use uuid::Uuid;

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

        let (outgoing_tx, mut outgoing_rx) = channel::<(Bytes, Address<'static>)>(2);
        let (mut incoming_tx, incoming_rx) =
            channel::<anyhow::Result<(Bytes, Address<'static>)>>(2);

        let (upstream_sink, upstream_stream) =
            upstream.to_sink_stream().to_connected(upstream_addr);

        let (mut msg_sink, mut msg_stream) = proto::new_message_sink_stream(
            upstream_sink.with(move |b: Bytes| {
                tx.inc(b.len());
                ready(Ok(b))
            }),
            upstream_stream.inspect_ok(move |data| rx.inc(data.len())),
        );

        // Listening for messages
        spawn(async move {
            // First wait for the ESTABLISH message
            let msg = match msg_stream.next().timeout(Duration::from_secs(1)).await {
                None => {
                    let _ = incoming_tx
                        .send(Err(anyhow!("Timeout waiting for first message")))
                        .await;
                    return;
                }
                Some(Some(Err(e))) => {
                    let _ = incoming_tx.send(Err(e)).await;
                    return;
                }
                Some(None) => {
                    let _ = incoming_tx
                        .send(Err(anyhow!("Message stream closed")))
                        .await;
                    return;
                }
                Some(Some(Ok(v))) => v,
            };
            log::debug!("Receive first reply message: {msg:?}");

            let (conn_id, initial_reply) = match msg {
                Message::Establish {
                    uuid: received_uuid,
                    conn_id,
                    initial_reply,
                } => {
                    if uuid.as_ref() != received_uuid.as_ref() {
                        let _ = incoming_tx
                            .send(Err(anyhow!(
                                "First replying message's UUID doesn't match the requested one"
                            )))
                            .await;
                        return;
                    }
                    (conn_id, initial_reply)
                }
                _ => {
                    let _ = incoming_tx
                        .send(Err(anyhow!("Invalid first message: {msg:?}")))
                        .await;
                    return;
                }
            };

            log::debug!("Connection established");
            if let Some((data, addr)) = initial_reply {
                let _ = incoming_tx.send(Ok((data, addr.into()))).await;
            }

            if incoming_tx.is_closed() {
                log::debug!("No further packet is needed. UDP conn closed");
                return;
            }

            let upload_task: Task<anyhow::Result<()>> = spawn(async move {
                while let Some((data, _)) = outgoing_rx.next().await {
                    let _ = msg_sink
                        .send(Message::Data {
                            conn_id: Some(conn_id),
                            addr: None,
                            payload: data.into(),
                            enc_nonce: None,
                        })
                        .await?;
                }
                Ok(())
            });
            let download_task: Task<anyhow::Result<()>> = spawn(async move {
                while let Some(d) = msg_stream.next().await {
                    match d? {
                        Message::Data { addr, payload, .. } => {
                            incoming_tx
                                .send(Ok((payload.into(), addr.unwrap_or(initial_dst).into())))
                                .await?;
                        }
                        m => {
                            log::warn!("Invalid message {m:?} received after ESTABLISH");
                        }
                    }
                }
                Ok(())
            });

            if let Err(e) = race(upload_task, download_task).await {
                log::error!("Error serving UDP: {e:?}");
            } else {
                log::debug!("UDP connection closed");
            }
        })
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
    use crate::{protocol::test::test_protocol_udp, test::create_udp_socket};
    use smol::{block_on, spawn};

    #[test]
    fn udpman_works() {
        std::env::set_var("RUST_LOG", "debug");
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
