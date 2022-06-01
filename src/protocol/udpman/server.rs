use std::{
    collections::VecDeque,
    future::ready,
    net::SocketAddr,
    sync::{Arc, Weak},
    time::Duration,
};

use super::proto::Message;
use crate::{
    io::{bind_udp, get_one_off_udp_query_timeout, Timer, UdpSocketExt},
    utils::{new_vec_for_udp, race, VecExt},
};
use anyhow::{bail, Context};
use async_net::UdpSocket;
use bytes::Bytes;
use futures::{
    channel::mpsc::{channel, Sender},
    select, FutureExt, Sink, SinkExt, Stream, StreamExt,
};
use parking_lot::{Mutex, RwLock};
use scopeguard::defer;
use smol::{spawn, Task};
use smol_timeout::TimeoutExt;

pub async fn serve_socket(socket: UdpSocket) -> anyhow::Result<()> {
    let (sink, stream) = socket.to_sink_stream().split();
    serve(
        Box::pin(
            sink.sink_map_err(anyhow::Error::from).with(
                |(data, addr): (Message<'static>, SocketAddr)| async move {
                    Ok((data.to_bytes()?, addr))
                },
            ),
        ),
        Box::pin(stream.filter_map(|item| {
            let (data, addr) = match item {
                Ok(v) => v,
                Err(e) => return ready(Some(Err(e))),
            };

            let msg = match Message::parse(data) {
                Ok(v) => v,
                Err(e) => {
                    log::error!("Error parsing message: {e:?}");
                    return ready(None);
                }
            };

            ready(Some(Ok((msg, addr))))
        })),
    )
    .await
}

pub async fn serve(
    mut sink: impl Sink<(Message<'static>, SocketAddr), Error = anyhow::Error> + Unpin + Send + 'static,
    mut stream: impl Stream<Item = anyhow::Result<(Message<'static>, SocketAddr)>>
        + Unpin
        + Send
        + 'static,
) -> anyhow::Result<()> {
    let connections: Arc<RwLock<ConnMap>> = Default::default();
    let (sink_tx, mut sink_rx) = channel::<(Message<'static>, SocketAddr)>(20);

    let task1: Task<anyhow::Result<()>> = {
        let connections = connections.clone();
        spawn(async move {
            while let Some(item) = stream.next().await {
                let (msg, from) = match item {
                    Ok(v) => v,
                    Err(e) => {
                        log::error!("Error receiving message: {e:?}");
                        continue;
                    }
                };
                log::debug!("Received {msg:?} from {from}");

                match msg {
                    Message::Connect {
                        uuid,
                        initial_data,
                        dst,
                        initial_data_nonce: None,
                    } => {
                        let mut map = connections.write();
                        let conn_id = match map.find_available_id() {
                            Some(v) => v,
                            None => {
                                log::warn!("Unable to find connection id for client {from}");
                                continue;
                            }
                        };
                        let conn = match Conn::new(
                            uuid.into(),
                            initial_data.into(),
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
                        map.insert(conn);
                    }
                    Message::Data {
                        conn_id: Some(conn_id),
                        payload,
                        ..
                    } => {
                        let send_result = if let Some(conn) = connections.read().get(conn_id) {
                            conn.incoming_tx.lock().try_send(payload.into())
                        } else {
                            log::warn!("Connection with ID {conn_id} does not exist");
                            continue;
                        };

                        match send_result {
                            Err(e) => {
                                log::error!("Error sending buf to conn_id {conn_id}: {e:?}");
                                if e.is_disconnected() {
                                    let _ = connections.write().remove(conn_id);
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
            Ok(())
        })
    };

    let task2 = spawn(async move {
        while let Some(a) = sink_rx.next().await {
            log::debug!("Sending Message({:?}) to {}", a.0, a.1);
            sink.send(a).await?;
        }
        anyhow::Result::<()>::Ok(())
    });

    let r = race(task1, task2).await;
    log::info!("Serve stopped at result: {r:?}");
    r
}

struct Conn {
    id: u16,
    incoming_tx: Mutex<Sender<Bytes>>,
    _task: Task<anyhow::Result<()>>,
}

#[derive(Default)]
struct ConnMap(VecDeque<Conn>);

impl ConnMap {
    pub fn insert(&mut self, c: Conn) {
        match self.0.binary_search_by_key(&c.id, |c| c.id) {
            Ok(p) => {
                self.0[p] = c;
            }
            Err(index) => {
                self.0.insert(index, c);
            }
        };
    }

    pub fn get(&self, id: u16) -> Option<&Conn> {
        self.0
            .binary_search_by_key(&id, |c| c.id)
            .ok()
            .map(|i| &self.0[i])
    }

    pub fn remove(&mut self, id: u16) -> Option<Conn> {
        self.0
            .binary_search_by_key(&id, |c| c.id)
            .ok()
            .and_then(|i| self.0.remove(i))
    }

    pub fn find_available_id(&self) -> Option<u16> {
        let max_key = self.0.back().map(|v| v.id).unwrap_or_default();

        // Can we simply take next item? (No wrapping)
        if let Some(next_key) = max_key.checked_add(1) {
            return Some(next_key);
        }

        // Start from beginning
        let mut expecting = 0;

        for conn in &self.0 {
            if expecting != conn.id {
                return Some(expecting);
            }
            expecting = expecting.checked_add(1)?;
        }

        None
    }
}

impl Conn {
    pub fn new(
        uuid: Bytes,
        initial_data: Bytes,
        src: SocketAddr,
        dst: SocketAddr,
        conn_id: u16,
        mut outgoing_tx: Sender<(Message<'static>, SocketAddr)>,
        connections: Weak<RwLock<ConnMap>>,
    ) -> anyhow::Result<Self> {
        log::debug!("Created UDP Connection(id = {conn_id}), src = {src}, dst = {dst}");
        let (incoming_tx, mut incoming_rx) = channel(10);
        let _task = spawn(async move {
            defer! {
                if let Some(conns) = connections.upgrade() {
                    log::debug!("UDP Conn(id={conn_id}) deleted");
                    let _ = conns.write().remove(conn_id);
                }
            }

            let upstream = bind_udp(matches!(dst, SocketAddr::V4(_))).await?;
            upstream
                .send_to(initial_data.as_ref(), dst)
                .await
                .with_context(|| format!("Sending initial data {dst}"))?;

            // Try to receive initial message within 500ms
            let mut buf = new_vec_for_udp();
            let initial_reply = match upstream
                .recv_from(&mut buf)
                .timeout(Duration::from_millis(500))
                .await
            {
                None => None,
                Some(Ok((len, addr))) => {
                    buf.set_len_uninit(len);
                    Some((Bytes::from(buf), addr))
                }
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

            if get_one_off_udp_query_timeout(&dst.into()).is_some() {
                return Ok(());
            }

            let (mut upstream_sink, mut upstream_stream) = upstream.to_sink_stream().split();
            let timer = Timer::new(Duration::from_secs(60));

            let upload_task = {
                let timer = timer.clone();
                spawn(async move {
                    while let Some(data) = incoming_rx.next().await {
                        upstream_sink.send((data, dst)).await?;
                        timer.reset();
                    }
                    anyhow::Result::<()>::Ok(())
                })
            };

            let download_task = {
                let timer = timer.clone();
                spawn(async move {
                    while let Some(p) = upstream_stream.next().await {
                        let (data, addr) = p?;
                        outgoing_tx
                            .send((
                                Message::Data {
                                    conn_id: None,
                                    addr: if addr == dst { None } else { Some(addr) },
                                    payload: data.into(),
                                    enc_nonce: None,
                                },
                                src,
                            ))
                            .await?;
                        timer.reset();
                    }
                    anyhow::Result::<()>::Ok(())
                })
            };

            select! {
                v1 = upload_task.fuse() => v1,
                v2 = download_task.fuse() => v2,
                _ = timer.fuse() => bail!("Timeout"),
            }
        });

        Ok(Self {
            id: conn_id,
            incoming_tx: Mutex::new(incoming_tx),
            _task,
        })
    }
}
