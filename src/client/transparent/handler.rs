use std::{borrow::Cow, collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use crate::{
    config::ClientConfig,
    protocol::Protocol,
    proxy::protocol::ProxyRequest,
    rt::{
        mpsc::{channel, Sender},
        spawn, Task,
    },
    socks5::Address,
};
use anyhow::Context;
use bytes::Bytes;
use futures::{future::ready, SinkExt, StreamExt};
use futures_util::{select, FutureExt};

use super::super::ClientStatistics;
use super::utils::bind_transparent_udp_for_reciving;

struct UdpSession {
    tx: Sender<Bytes>,
    _task: Task<anyhow::Result<()>>,
}

#[derive(PartialEq, Eq, Hash, Debug)]
struct UdpSessionKey {
    src: SocketAddr,
    dst: SocketAddr,
}

pub async fn serve_udp_transparent_proxy(
    addr: SocketAddr,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<Task<anyhow::Result<()>>> {
    let mut socket = bind_transparent_udp_for_reciving(addr).context("Binding UDP socket")?;
    log::info!("Started UDP transparent proxy at {addr}");
    Ok(spawn(async move {
        let mut sessions: HashMap<UdpSessionKey, UdpSession> = Default::default();
        let (cleanup_tx, mut cleanup_rx) = channel::<UdpSessionKey>(2);

        loop {
            let (buf, src, dst) = select! {
                k = cleanup_rx.next().fuse() => {
                    let k = k.context("Timeout channel closed")?;
                    log::info!("UDP session {k:?} timeout");
                    sessions.remove(&k);
                    continue;
                }

                r = socket.next().fuse() => {
                    match r {
                        Some(v) => v,
                        None => {
                            return Ok(())
                        }
                    }
                }
            };

            log::debug!("TProxy received {} from {src}, orig dst = {dst}", buf.len());
            let key = UdpSessionKey { src, dst };
            match sessions.get_mut(&key) {
                Some(s) => match s.tx.try_send(buf) {
                    Ok(_) => {}
                    Err(v) if v.is_disconnected() => {
                        let key = UdpSessionKey { src, dst };
                        log::debug!("Session {key:?} closed");
                        sessions.remove(&key);
                    }
                    Err(_) => {}
                },
                None => {
                    let session = UdpSession::new(
                        src,
                        dst,
                        config.clone(),
                        stats.clone(),
                        cleanup_tx.clone(),
                        buf,
                    );
                    sessions.insert(key, session);
                }
            };
        }
    }))
}

impl UdpSession {
    pub fn new(
        src: SocketAddr,
        dst: SocketAddr,
        config: Arc<ClientConfig>,
        stats: Arc<ClientStatistics>,
        mut clean_up: Sender<UdpSessionKey>,
        initial_data: Bytes,
    ) -> Self {
        let (tx, rx) = channel(10);
        let dst_addr: Address = dst.into();
        let _task = spawn(async move {
            let req = ProxyRequest::UDP {
                initial_dst: dst_addr.clone(),
                initial_data: Cow::Borrowed(&initial_data),
            };

            let mut upstreams = config.find_best_upstream(&req, &stats, &dst_addr);
            let mut result = Ok(());
            while let Some((name, upstream)) = upstreams.pop() {
                log::debug!("Trying upstream {name} for UDP://{dst_addr}");
                let (sink, stream) = match upstream
                    .protocol
                    .new_dgram_conn(&req)
                    .await
                    .with_context(|| format!("Creating upstream dgram for {req:?}"))
                {
                    Ok(v) => v,
                    Err(e) => {
                        result = Err(e.into());
                        continue;
                    }
                };

                result = super::udp_proxy::serve_udp_on_dgram(
                    sink.with(|(buf, addr)| ready(anyhow::Result::Ok((buf, Address::from(addr))))),
                    stream.filter_map(|(buf, addr)| {
                        ready(match addr {
                            Address::IP(addr) => Some((buf, addr)),
                            _ => None,
                        })
                    }),
                    src,
                    dst,
                    rx,
                    initial_data.clone(),
                    Duration::from_secs(60),
                )
                .await
                .with_context(|| format!("serving UDP://{dst_addr} on {name}"));
                break;
            }

            clean_up.send(UdpSessionKey { src, dst }).await?;
            result
        });

        Self { tx, _task }
    }
}
