use std::{borrow::Cow, collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use crate::{
    config::ClientConfig,
    io::bind_udp,
    proxy::protocol::ProxyRequest,
    rt::{
        mpsc::{bounded, Sender, TrySendError},
        spawn, Task,
    },
    socks5::Address,
};
use anyhow::Context;
use bytes::Bytes;
use futures::StreamExt;
use futures_util::{select, FutureExt};

use super::super::ClientStatistics;
use super::utils::bind_transparent_udp;

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
    let mut socket = bind_transparent_udp(addr).context("Binding UDP socket")?;
    log::info!("Started UDP transparent proxy at {addr}");
    Ok(spawn(async move {
        let mut sessions: HashMap<UdpSessionKey, UdpSession> = Default::default();
        let (cleanup_tx, mut cleanup_rx) = bounded::<UdpSessionKey>(2);

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
                    Err(TrySendError::Closed(_)) => {
                        let key = UdpSessionKey { src, dst };
                        log::debug!("Session {key:?} closed");
                        sessions.remove(&key);
                    }
                    Err(TrySendError::Full(_)) => {}
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
        clean_up: Sender<UdpSessionKey>,
        initial_data: Bytes,
    ) -> Self {
        let (tx, rx) = bounded(10);
        let dst_addr: Address = dst.into();
        let _task = spawn(async move {
            todo!()
            // let upstream = match request_best_upstream(
            //     &config,
            //     &stats,
            //     &dst_addr,
            //     &ProxyRequest::UDP {
            //         initial_dst: dst_addr.clone(),
            //         initial_data: Cow::Borrowed(&initial_data),
            //     },
            // )
            // .await
            // {
            //     Ok((_, stream, stats)) => Some((stream, stats)),
            //     Err(err) => {
            //         log::error!("Error requesting upstream: {err:?}");
            //         None
            //     }
            // };

            // let result = if let Some((upstream, stats)) = upstream {
            //     // Proxy through upstream
            //     super::proxy::serve_udp_on_stream(
            //         src,
            //         dst_addr,
            //         rx,
            //         upstream,
            //         stats,
            //         Duration::from_secs(60),
            //     )
            //     .await
            // } else if config.allow_direct(&dst_addr) {
            //     // Direct connect
            //     super::udp_proxy::serve_udp_on_dgram(
            //         bind_udp(matches!(dst, SocketAddr::V4(_)))
            //             .await
            //             .with_context(|| format!("Binding direct socket for client {src}"))?,
            //         src,
            //         dst,
            //         rx,
            //         initial_data,
            //         Duration::from_secs(60),
            //     )
            //     .await
            // } else {
            //     Ok(())
            // };

            // clean_up.send(UdpSessionKey { src, dst }).await?;
            // result
        });

        Self { tx, _task }
    }
}
