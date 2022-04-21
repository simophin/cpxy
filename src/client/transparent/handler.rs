use std::{borrow::Cow, collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use crate::{
    config::ClientConfig,
    proxy::protocol::ProxyRequest,
    rt::{
        mpsc::{bounded, Sender, TrySendError},
        spawn, Task,
    },
    socks5::Address,
    utils::{new_vec_for_udp, VecExt},
};
use anyhow::Context;
use futures::StreamExt;
use futures_util::{select, FutureExt};

use super::super::{utils::request_best_upstream, ClientStatistics};
use super::TransparentUdpSocket;

struct UdpSession {
    tx: Sender<Vec<u8>>,
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
    let socket = super::utils::bind_transparent_udp(addr).context("Binding UDP socket")?;
    log::info!("Started UDP transparent proxy at {addr}");
    Ok(spawn(async move {
        let mut sessions: HashMap<UdpSessionKey, UdpSession> = Default::default();
        let (cleanup_tx, mut cleanup_rx) = bounded::<UdpSessionKey>(2);

        let mut buf = new_vec_for_udp();

        loop {
            let ((len, src), dst) = select! {
                k = cleanup_rx.next().fuse() => {
                    let k = k.context("Timeout channel closed")?;
                    log::info!("UDP session {k:?} timeout");
                    sessions.remove(&k);
                    continue;
                }

                r = socket.recv_from(&mut buf).fuse() => {
                    match r {
                        Ok(v) => v,
                        Err(e) => {
                            log::error!("Error receiving TProxy packet: {e:?}");
                            return Err(e.into());
                        }
                    }
                }
            };

            buf.set_len_uninit(len);

            log::debug!("TProxy received {len} from {src}, orig dst = {dst}");
            let key = UdpSessionKey { src, dst };
            buf = match sessions.get_mut(&key) {
                Some(s) => match s.tx.try_send(buf) {
                    Ok(_) => new_vec_for_udp(),
                    Err(TrySendError::Closed(b)) => {
                        let key = UdpSessionKey { src, dst };
                        log::debug!("Session {key:?} closed");
                        sessions.remove(&key);
                        b
                    }
                    Err(TrySendError::Full(b)) => b,
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
                    new_vec_for_udp()
                }
            };

            buf.set_len_to_capacity();
        }
    }))
}

impl UdpSession {
    pub fn new(
        src: SocketAddr,
        dst: SocketAddr,
        config: Arc<ClientConfig>,
        stats: Arc<ClientStatistics>,
        clean_up: Sender<UdpSessionKey>,
        initial_data: Vec<u8>,
    ) -> Self {
        let (tx, rx) = bounded(10);
        let dst_addr: Address = dst.into();
        let _task = spawn(async move {
            let upstream = match request_best_upstream(
                &config,
                &stats,
                &dst_addr,
                &ProxyRequest::UDP {
                    initial_dst: dst_addr.clone(),
                    initial_data: Cow::Borrowed(&initial_data),
                },
            )
            .await
            {
                Ok((_, stream, stats)) => Some((stream, stats)),
                Err(err) => {
                    log::error!("Error requesting upstream: {err:?}");
                    None
                }
            };

            let result = if let Some((upstream, stats)) = upstream {
                // Proxy through upstream
                super::proxy::serve_udp_with_upstream(
                    src,
                    dst_addr,
                    rx,
                    upstream,
                    stats,
                    Duration::from_secs(60),
                )
                .await
            } else if config.allow_direct(&dst_addr) {
                // Direct connect
                super::direct::serve_udp_direct(rx).await
            } else {
                Ok(())
            };

            clean_up.send(UdpSessionKey { src, dst }).await?;
            result
        });

        Self { tx, _task }
    }
}
