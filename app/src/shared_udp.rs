use anyhow::{anyhow, bail};
use futures_lite::StreamExt;
use std::{
    collections::HashMap,
    fmt::Debug,
    hash::Hash,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use smol::{
    channel::{bounded, Receiver, Sender},
    spawn, Task, Timer,
};

use crate::{buf::Buf, io::UdpSocket};

struct QueueItem {
    sender: Sender<(Buf, SocketAddr)>,
    time: Instant,
}

pub struct SharedClient<ID> {
    socket: Arc<UdpSocket>,
    waiting_queue: Arc<Mutex<HashMap<ID, QueueItem>>>,
    task: Task<anyhow::Result<()>>,
}

pub trait IDParser {
    type IDType;

    fn parse_id(buf: &[u8], addr: Option<&SocketAddr>) -> anyhow::Result<Self::IDType>;
}

impl<ID: Send + Sync + Eq + Hash + Debug + 'static> SharedClient<ID> {
    pub fn new<ToID: IDParser<IDType = ID> + Send + Sync + 'static>() -> anyhow::Result<Self> {
        let socket = Arc::new(UdpSocket::bind_sync(&SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            0,
        )))?);
        let waiting_queue = Arc::new(Mutex::new(HashMap::<ID, QueueItem>::new()));
        let task = {
            let socket = socket.clone();
            let waiting_queue = waiting_queue.clone();
            spawn(async move {
                let mut buf = Buf::new_for_udp();
                loop {
                    let (len, addr) = socket.recv_from(&mut buf).await?;
                    buf.set_len(len);
                    match ToID::parse_id(&buf, Some(&addr)) {
                        Ok(id) => {
                            let sender = waiting_queue
                                .lock()
                                .ok()
                                .and_then(|mut map| map.remove(&id));
                            if let Some(sender) = sender {
                                sender.sender.send((buf, addr)).await?;
                                buf = Buf::new_for_udp();
                            }
                        }
                        Err(e) => {
                            log::error!("Error parsing message from {addr}: {e:?}");
                            buf.set_len(buf.capacity());
                        }
                    }
                }
            })
        };
        Ok(Self {
            socket,
            waiting_queue,
            task,
        })
    }

    pub fn register(
        &self,
        id: ID,
    ) -> anyhow::Result<(Arc<UdpSocket>, Receiver<(Buf, SocketAddr)>)> {
        let (tx, rx) = bounded(1);
        let mut g = self
            .waiting_queue
            .lock()
            .map_err(|_| anyhow!("Error locking queue"))?;
        if g.contains_key(&id) {
            bail!("ID {id:?} exist");
        }

        g.insert(
            id,
            QueueItem {
                sender: tx,
                time: Instant::now(),
            },
        );
        Ok((self.socket.clone(), rx))
    }

    pub fn unregister(&self, id: &ID) {
        if let Ok(mut q) = self.waiting_queue.lock() {
            q.remove(id);
        }
    }
}
