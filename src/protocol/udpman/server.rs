use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Weak},
    time::Duration,
};

use super::proto::{BytesRef, Message};
use crate::{
    io::{bind_udp, is_one_off_udp_query, UdpSocketExt},
    rt::{
        mpsc::{channel, Sender},
        net::UdpSocket,
        spawn, Task, TimeoutExt,
    },
    utils::race,
};
use anyhow::{bail, Context};
use bytes::Bytes;
use futures::{select, FutureExt, Sink, SinkExt, Stream, StreamExt};
use num_traits::PrimInt;
use parking_lot::{Mutex, RwLock};
use scopeguard::defer;

pub async fn serve_socket(socket: UdpSocket) -> anyhow::Result<()> {
    let (sink, stream) = socket.to_sink_stream().split();
    serve(
        sink.sink_map_err(anyhow::Error::from).with(
            |(data, addr): (Message<'static>, SocketAddr)| async move {
                Ok((data.to_bytes()?, addr))
            },
        ),
        Box::pin(stream.filter_map(|(data, addr)| async move {
            Some((Message::parse(data).ok()?, addr))
        })),
    ).await
}

pub async fn serve(
    sink: impl Sink<(Message<'static>, SocketAddr), Error = anyhow::Error> + Send + 'static,
    mut stream: impl Stream<Item = (Message<'static>, SocketAddr)> + Unpin + Send + 'static,
) -> anyhow::Result<()> {
    let connections: Arc<RwLock<BTreeMap<u16, Conn>>> = Default::default();
    let (sink_tx, sink_rx) = channel::<(Message<'static>, SocketAddr)>(20);

    let task1 = {
        let connections = connections.clone();
        spawn(async move {
            while let Some((msg, from)) = stream.next().await {
                log::debug!("Received {msg:?} from {from}");

                match msg {
                    Message::Connect {
                        uuid,
                        initial_data,
                        dst,
                        initial_data_nonce: None,
                    } => {
                        let mut map = connections.write();
                        let conn_id = match find_available_conn_id(&map) {
                            Some(v) => v,
                            None => {
                                log::warn!("Unable to find connection id for client {from}");
                                continue;
                            }
                        };
                        let conn = match Conn::new(
                            uuid,
                            initial_data,
                            from,
                            dst,
                            conn_id,
                            sink_tx.clone(),
                            Arc::downgrade(&connections),
                        ) {
                            Ok(v) => v,
                            Err(e) => {
                                log::error!("Unable to create connection for client {from}: {e:?}");
                                continue;
                            }
                        };
                        map.insert(conn_id, conn);
                    }
                    Message::Data {
                        conn_id: Some(conn_id),
                        payload,
                        ..
                    } => {
                        let send_result = if let Some(conn) = connections.read().get(&conn_id) {
                            conn.incoming_tx.lock().try_send(payload.into())
                        } else {
                            log::warn!("Connection with ID {conn_id} does not exist");
                            continue;
                        };

                        match send_result {
                            Err(e) => {
                                log::error!("Error sending buf to conn_id {conn_id}: {e:?}");
                                if e.is_disconnected() {
                                    let _ = connections.write().remove(&conn_id);
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        log::debug!("Unsupported message {msg:?}");
                    }
                }
            }
        })
    };

    let task2 = spawn(
        sink_rx
            .map(|a| {
                log::debug!("Sending Message({:?}) to {}", a.0, a.1);
                Ok(a)
            })
            .forward(sink),
    );
    select! {
        _ = task1.fuse() => Ok(()),
        v = task2.fuse() => v,
    }
}

struct Conn {
    incoming_tx: Mutex<Sender<Bytes>>,
    _task: Task<anyhow::Result<()>>,
}

impl Conn {
    pub fn new(
        uuid: BytesRef<'static>,
        initial_data: BytesRef<'static>,
        src: SocketAddr,
        dst: SocketAddr,
        conn_id: u16,
        mut outgoing_tx: Sender<(Message<'static>, SocketAddr)>,
        connections: Weak<RwLock<BTreeMap<u16, Conn>>>,
    ) -> anyhow::Result<Self> {
        log::debug!("Created UDP Connection(id = {conn_id}), src = {src}, dst = {dst}");
        let (incoming_tx, incoming_rx) = channel(10);
        let _task = spawn(async move {
            defer! {
                if let Some(conns) = connections.upgrade() {
                    log::debug!("UDP Conn(id={conn_id}) deleted");
                    let _ = conns.write().remove(&conn_id);
                }
            }

            let upstream = bind_udp(matches!(dst, SocketAddr::V4(_))).await?;
            upstream
                .send_to(initial_data.as_ref(), dst)
                .await
                .with_context(|| format!("Sending initial data {dst}"))?;

            // Try to receive initial message within 500ms
            let initial_reply = match upstream
                .recv_bytes_from()
                .timeout(Duration::from_millis(500))
                .await
            {
                None => None,
                Some(Ok(v)) => Some(v),
                Some(Err(e)) => bail!("Error receiving initial data: {e:?}"),
            };

            // Send Establish message
            outgoing_tx
                .send((
                    Message::Establish {
                        uuid: uuid.into(),
                        conn_id,
                        initial_reply,
                    },
                    src,
                ))
                .await
                .context("Sending established message")?;

            if is_one_off_udp_query(&dst.into()) {
                return Ok(());
            }

            let (upstream_sink, upstream_stream) = upstream.to_sink_stream().split();

            let upload_task = spawn(
                incoming_rx
                    .map(move |data| Ok((data, dst)))
                    .forward(upstream_sink.sink_map_err(anyhow::Error::from)),
            );

            let download_task = spawn(
                upstream_stream
                    .map(move |(data, addr)| {
                        Ok((
                            Message::Data {
                                conn_id: None,
                                addr: if addr == dst { None } else { Some(addr) },
                                payload: BytesRef::Bytes(data),
                                enc_nonce: None,
                            },
                            src,
                        ))
                    })
                    .forward(outgoing_tx.sink_map_err(anyhow::Error::from)),
            );

            race(upload_task, download_task).await
        });

        Ok(Self {
            incoming_tx: Mutex::new(incoming_tx),
            _task,
        })
    }
}

fn find_available_conn_id<K: PrimInt + Ord, V>(map: &BTreeMap<K, V>) -> Option<K> {
    let max_key = match map.last_key_value() {
        Some(v) => v.0,
        None => return K::from(0),
    };

    // Can we simply take next item? (No wrapping)
    let inc = K::from(1)?;
    if let Some(next_key) = max_key.checked_add(&inc) {
        return Some(next_key);
    }

    // Start from beginning
    let mut expecting = K::from(0)?;

    for (k, _) in map {
        if expecting != *k {
            return Some(expecting);
        }
        expecting = expecting.checked_add(&inc)?;
    }

    None
}
