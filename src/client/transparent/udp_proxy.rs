use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{bail, Context};
use bytes::Bytes;
use futures::{select, FutureExt, Sink, SinkExt, Stream, StreamExt};
use parking_lot::Mutex;

use crate::rt::{
    mpsc::{channel, Receiver},
    spawn, Task, TimeoutExt,
};

use super::utils::bind_transparent_udp_for_sending;

pub async fn serve_udp_on_dgram(
    mut upstream: impl Stream<Item = (Bytes, SocketAddr)>
        + Sink<(Bytes, SocketAddr), Error = std::io::Error>
        + Unpin
        + Send
        + Sync
        + 'static,
    src: SocketAddr,
    dst: SocketAddr,
    mut rx: Receiver<Bytes>,
    initial_data: Bytes,
    timeout: Duration,
) -> anyhow::Result<()> {
    if !initial_data.is_empty() {
        upstream
            .feed((initial_data, dst))
            .await
            .with_context(|| format!("Sending initial data to {dst} for {src}"))?;
    }

    let (mut upstream_w, mut upstream_r) = upstream.split();

    let should_close_after_receive = dst.port() == 53;
    let last_upstream_addr = Arc::new(Mutex::new(None));
    let (mut active_tx, mut active_rx) = channel::<()>(5);

    // SRC -> UPSTRAEM
    let task1: Task<anyhow::Result<()>> = {
        let last_upstream_addr = last_upstream_addr.clone();
        let mut active_tx = active_tx.clone();
        spawn(async move {
            while let Some(data) = rx.next().await {
                let addr = last_upstream_addr.lock().unwrap_or(dst);
                let _ = active_tx.try_send(());
                upstream_w
                    .feed((data, addr))
                    .await
                    .with_context(|| format!("Sending dgram to {addr} from client {src}"))?;
            }
            Ok(())
        })
    };

    // UPSTREAM -> SRC
    let task2: Task<anyhow::Result<()>> = spawn(async move {
        let mut sockets = HashMap::<SocketAddr, _>::new();
        loop {
            let (buf, from) = upstream_r
                .next()
                .await
                .with_context(|| format!("Receiving datagram for client {src}"))?;

            let _ = active_tx.try_send(());
            last_upstream_addr.lock().replace(from);

            if should_close_after_receive {
                bind_transparent_udp_for_sending(from)
                    .with_context(|| {
                        format!("Binding returning socket on {from} for client {src}")
                    })?
                    .feed((buf, src))
                    .await
                    .with_context(|| format!("Responding to {src}"))?;
                break Ok(());
            }

            let socket = sockets.get_mut(&from);
            if socket.is_none() {
                let mut s = bind_transparent_udp_for_sending(from).with_context(|| {
                    format!("Binding returning socket on {from} for client {src}")
                })?;
                s.feed((buf, src))
                    .await
                    .with_context(|| format!("Responding to {src}"))?;
                sockets.insert(from, s);
            } else {
                socket
                    .unwrap()
                    .feed((buf, src))
                    .await
                    .with_context(|| format!("Responding to {src}"))?;
            }
        }
    });

    let mut task1 = task1.fuse();
    let mut task2 = task2.fuse();

    loop {
        select! {
            t1 = task1 => return t1,
            t2 = task2 => return t2,
            v = active_rx.next().timeout(timeout).fuse() => {
                if v.is_none() {
                    bail!("{src} Timeout");
                }
            },
        }
    }
}
