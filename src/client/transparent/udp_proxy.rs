use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Context;
use bytes::Bytes;
use futures::{future::ready, select, FutureExt, Sink, SinkExt, Stream, StreamExt};
use parking_lot::Mutex;

use crate::{
    io::{is_one_off_udp_query, Timer},
    rt::{mpsc::Receiver, spawn, Task, TimeoutExt},
};

use super::utils::bind_transparent_udp_for_sending;

pub async fn serve_udp_on_dgram(
    mut upstream_sink: impl Sink<(Bytes, SocketAddr), Error = anyhow::Error> + Unpin + Send + 'static,
    mut upstream_stream: impl Stream<Item = (Bytes, SocketAddr)> + Unpin + Send + 'static,
    src: SocketAddr,
    dst: SocketAddr,
    rx: Receiver<Bytes>,
    initial_data: Bytes,
    timeout: Duration,
) -> anyhow::Result<()> {
    upstream_sink
        .send((initial_data, dst))
        .await
        .with_context(|| format!("Sending initial data to {dst} for {src}"))?;

    if is_one_off_udp_query(&dst.into()) {
        let data = upstream_stream
            .next()
            .timeout(timeout)
            .await
            .context("Timeout waiting for one off query response")?
            .context("Unexpected EOF from upstream")?;
        return bind_transparent_udp_for_sending(dst)
            .context("Binding TProxy for sending")?
            .send(data)
            .await
            .context("Sending reply back to one off query");
    }

    let timer = Timer::new(timeout);
    let last_upstream_addr = Arc::new(Mutex::new(None));

    // SRC -> UPSTRAEM
    let upload_task = {
        let last_upstream_addr = last_upstream_addr.clone();
        let timer = timer.clone();

        spawn(
            rx.filter_map(move |data| {
                ready(
                    last_upstream_addr
                        .lock()
                        .map(|addr| anyhow::Result::Ok((data, addr))),
                )
            })
            .inspect(move |_| timer.reset())
            .forward(upstream_sink),
        )
    };

    // UPSTREAM -> SRC
    let download_task: Task<anyhow::Result<()>> = {
        let timer = timer.clone();
        spawn(async move {
            let mut sockets = HashMap::<SocketAddr, _>::new();
            loop {
                let (buf, from) = upstream_stream
                    .next()
                    .await
                    .with_context(|| format!("Receiving datagram for client {src}"))?;

                timer.reset();
                last_upstream_addr.lock().replace(from);

                let socket = sockets.get_mut(&from);
                if socket.is_none() {
                    let mut s = bind_transparent_udp_for_sending(from).with_context(|| {
                        format!("Binding returning socket on {from} for client {src}")
                    })?;
                    s.send((buf, src))
                        .await
                        .with_context(|| format!("Responding to {src}"))?;
                    sockets.insert(from, s);
                } else {
                    socket
                        .unwrap()
                        .send((buf, src))
                        .await
                        .with_context(|| format!("Responding to {src}"))?;
                }
            }
        })
    };

    select! {
        _ = upload_task.fuse() => Ok(()),
        _ = download_task.fuse() => Ok(()),
        _ = timer.fuse() => Ok(())
    }
}
