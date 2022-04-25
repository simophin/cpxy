use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{bail, Context};
use bytes::Bytes;
use futures::{select, FutureExt, Sink, Stream, StreamExt};
use parking_lot::Mutex;

use crate::rt::{
    mpsc::{channel, Receiver},
    spawn, Task, TimeoutExt,
};

pub async fn serve_udp_on_dgram(
    upstream: impl Stream<Item = (Bytes, SocketAddr, SocketAddr)>
        + Sink<(Bytes, SocketAddr, SocketAddr), Error = anyhow::Error>
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
    let upstream = Arc::new(upstream);

    if !initial_data.is_empty() {
        upstream
            .send_dgram(&initial_data, dst)
            .await
            .with_context(|| {
                format!(
                    "Sending initial data(len={}) to {dst} for {src}",
                    initial_data.len()
                )
            })?;
    }

    let should_close_after_receive = dst.port() == 53;
    let last_upstream_addr = Arc::new(Mutex::new(None));
    let (active_tx, mut active_rx) = channel::<()>(5);

    // SRC -> UPSTRAEM
    let task1: Task<anyhow::Result<()>> = {
        let upstream = upstream.clone();
        let last_upstream_addr = last_upstream_addr.clone();
        let active_tx = active_tx.clone();
        spawn(async move {
            while let Some(data) = rx.next().await {
                let addr = last_upstream_addr.lock().unwrap_or(dst);
                let _ = active_tx.try_send(());
                upstream
                    .send_dgram(&data, addr)
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
            let (buf, from) = upstream
                .recv_dgram()
                .await
                .with_context(|| format!("Receiving datagram for client {src}"))?;

            let _ = active_tx.try_send(());
            last_upstream_addr.lock().replace(from);

            if should_close_after_receive {
                bind_transparent_udp(from)
                    .with_context(|| {
                        format!("Binding returning socket on {from} for client {src}")
                    })?
                    .send_dgram(&buf, src)
                    .await
                    .with_context(|| format!("Responding to {src}"))?;
                break Ok(());
            }

            let socket = sockets.get(&from);
            if socket.is_none() {
                let s = bind_transparent_udp(from).with_context(|| {
                    format!("Binding returning socket on {from} for client {src}")
                })?;
                s.send_dgram(&buf, src)
                    .await
                    .with_context(|| format!("Responding to {src}"))?;
                sockets.insert(from, s);
            } else {
                socket
                    .unwrap()
                    .send_dgram(&buf, src)
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
