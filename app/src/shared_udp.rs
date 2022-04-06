use std::{
    collections::HashMap,
    hash::Hash,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use futures_lite::Stream;
use smol::{
    channel::{bounded, SendError, Sender},
    spawn, Task,
};

use crate::{buf::Buf, io::UdpSocket};

pub struct SharedUdpSocket<K> {
    socket: Arc<UdpSocket>,
    sharees: Arc<RwLock<HashMap<K, Sender<(Buf, SocketAddr)>>>>,
    _task: Task<anyhow::Result<()>>,
}

impl<K: Hash + Eq + Send + Sync + 'static> SharedUdpSocket<K> {
    pub fn new(
        socket: UdpSocket,
        packet_id: impl Fn(&[u8], &SocketAddr) -> Option<K> + Send + Sync + 'static,
    ) -> Self {
        let socket = Arc::new(socket);
        let sharees = Arc::new(RwLock::new(HashMap::new()));
        Self {
            socket: socket.clone(),
            sharees: sharees.clone(),
            _task: spawn(async move {
                let mut buf = Buf::new_for_udp();
                loop {
                    let (len, addr) = socket.recv_from(&mut buf).await?;
                    buf.set_len(len);

                    let id = match packet_id(&buf, &addr) {
                        Some(v) => v,
                        None => {
                            log::debug!("Unable to find packet id for packet from {addr}");
                            continue;
                        }
                    };

                    let tx = match sharees
                        .read()
                        .map_err(|_| anyhow::anyhow!("Error locking sharees"))?
                        .get(&id)
                        .map(|t| t.clone())
                    {
                        Some(v) => v,
                        None => {
                            log::debug!("Unable to find sender for packet from {addr}");
                            continue;
                        }
                    };

                    match tx.send((buf, addr)).await {
                        Ok(_) => buf = Buf::new_for_udp(),
                        Err(SendError(b)) => {
                            log::info!("Channel for {addr} is closed. Cleaning up");
                            sharees
                                .write()
                                .map_err(|_| anyhow::anyhow!("Error locking sharees"))?
                                .remove(&id);
                            buf = b.0;
                            buf.set_len(buf.capacity());
                        }
                    }
                }
            }),
        }
    }

    pub fn register(
        &self,
        k: K,
    ) -> (
        Arc<UdpSocket>,
        impl Stream<Item = (Buf, SocketAddr)> + Unpin + Send + Sync + 'static,
    ) {
        let (tx, rx) = bounded(2);
        self.sharees
            .write()
            .expect("To lock as write")
            .insert(k, tx);

        (self.socket.clone(), rx)
    }

    pub fn unregister(&self, k: K) {
        self.sharees.write().expect("To lock as write").remove(&k);
    }
}
