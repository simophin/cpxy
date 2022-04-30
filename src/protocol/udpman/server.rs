use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use super::proto::{BytesRef, Message};
use crate::rt::{
    mpsc::{channel, Sender, TrySendError},
    spawn, Task,
};
use bytes::Bytes;
use futures::{Sink, Stream, StreamExt};
use num_traits::PrimInt;
use parking_lot::{Mutex, RwLock};

pub async fn serve(
    sink: impl Sink<(Message<'static>, SocketAddr), Error = anyhow::Error>
        + Clone
        + Unpin
        + Send
        + 'static,
    mut stream: impl Stream<Item = (Message<'static>, SocketAddr)> + Unpin + Send + 'static,
) -> anyhow::Result<()> {
    let mut connections: Arc<RwLock<BTreeMap<u16, Conn>>> = Default::default();

    let _task1 = {
        let connections = connections.clone();
        spawn(async move {
            while let Some((msg, from)) = stream.next().await {
                log::debug!("Received {msg:?} from {from}");

                match msg {
                    Message::Connect {
                        uuid,
                        initial_data,
                        dst,
                    } => {
                        let mut map = connections.write();
                        let conn_id = match find_available_conn_id(&map) {
                            Some(v) => v,
                            None => {
                                log::warn!("Unable to find connection id for client {from}");
                                continue;
                            }
                        };
                        let conn = match Conn::new(uuid, initial_data, dst, conn_id, sink.clone()) {
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

    todo!()
}

struct Conn {
    incoming_tx: Mutex<Sender<Bytes>>,
}

impl Conn {
    pub fn new(
        uuid: BytesRef<'static>,
        initial_data: BytesRef<'static>,
        dst: SocketAddr,
        conn_id: u16,
        sink: impl Sink<(Message<'static>, SocketAddr), Error = anyhow::Error> + Unpin + Send + 'static,
    ) -> anyhow::Result<Self> {
        todo!()
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
