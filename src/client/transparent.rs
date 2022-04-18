use std::{
    borrow::Cow,
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    os::unix::prelude::AsRawFd,
    sync::Arc,
};

use crate::{
    buf::Buf,
    config::ClientConfig,
    proxy::protocol::ProxyRequest,
    rt::{
        mpsc::{bounded, Receiver, Sender, TrySendError},
        net::UdpSocket,
        spawn, Task,
    },
    socks5::Address,
};
use anyhow::{bail, Context};
use futures_lite::{AsyncRead, AsyncWrite, StreamExt};
use futures_util::{select, FutureExt};

use super::{utils::request_best_upstream, ClientStatistics, UpstreamStatistics};

struct UdpSession {
    tx: Sender<Buf>,
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
    let socket = Arc::new(UdpSocket::bind(&addr).await.context("Binding UDP socket")?);
    log::info!("Started UDP transparent proxy at {addr}");
    Ok(spawn(async move {
        let mut sessions: HashMap<UdpSessionKey, UdpSession> = Default::default();
        let (timeout_tx, mut timeout_rx) = bounded::<UdpSessionKey>(2);

        let mut buf = Buf::new_for_udp();
        loop {
            let ((len, src), dst) = select! {
                k = timeout_rx.next().fuse() => {
                    let k = k.context("Timeout channel closed")?;
                    log::info!("UDP session {k:?} timeout");
                    sessions.remove(&k);
                    continue;
                }

                r = recv_with_orig_dst(&socket, &mut buf).fuse() => {
                    r.context("Receiving tproxy proxy")?
                }
            };

            buf.set_len(len);

            log::debug!("TProxy received {len} from {src}, orig dst = {dst}");
            let key = UdpSessionKey { src, dst };
            buf = match sessions.get_mut(&key) {
                Some(s) => match s.tx.try_send(buf) {
                    Ok(_) => Buf::new_for_udp(),
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
                        socket.clone(),
                        config.clone(),
                        stats.clone(),
                        timeout_tx.clone(),
                        buf,
                    );
                    sessions.insert(key, session);
                    Buf::new_for_udp()
                }
            };

            buf.set_len(buf.capacity());
        }
    }))
}

async fn recv_with_orig_dst(
    socket: &UdpSocket,
    buf: &mut [u8],
) -> std::io::Result<((usize, SocketAddr), SocketAddr)> {
    Ok(((10, "123".parse().unwrap()), "456".parse().unwrap()))
}

impl UdpSession {
    pub fn new(
        src: SocketAddr,
        dst: SocketAddr,
        socket: Arc<UdpSocket>,
        config: Arc<ClientConfig>,
        stats: Arc<ClientStatistics>,
        timeout_tx: Sender<UdpSessionKey>,
        initial_data: Buf,
    ) -> Self {
        let (tx, rx) = bounded(10);
        let dst: Address = dst.into();
        let _task = spawn(async move {
            let upstream = match request_best_upstream(
                &config,
                &stats,
                &dst,
                &ProxyRequest::UDP {
                    initial_dst: dst.clone(),
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

            if let Some((upstream, stats)) = upstream {
                // Proxy through upstream
                copy_between_udp_and_upstream(rx, upstream, stats).await
            } else if config.allow_direct(&dst) {
                // Direct connect
                serve_udp_direct(rx).await
            } else {
                Ok(())
            }
        });

        Self { tx, _task }
    }
}

async fn copy_between_udp_and_upstream(
    mut rx: Receiver<Buf>,
    upstream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    stats: Option<&UpstreamStatistics>,
) -> anyhow::Result<()> {
    todo!()
}

async fn serve_udp_direct(mut rx: Receiver<Buf>) -> anyhow::Result<()> {
    todo!()
}
