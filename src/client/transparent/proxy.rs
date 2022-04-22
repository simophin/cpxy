use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::bail;
use futures::{select, AsyncRead, AsyncReadExt, AsyncWrite, FutureExt, StreamExt};

use crate::client::UpstreamStatistics;
use crate::io::DatagramSocket;
use crate::proxy::udp::{PacketReader, PacketWriter};
use crate::socks5::Address;

use crate::rt::{
    mpsc::{bounded, Receiver},
    spawn, Task, TimeoutExt,
};

pub async fn serve_udp_with_upstream<
    S: DatagramSocket<RecvType = ((usize, SocketAddr), SocketAddr)> + Unpin + Send + Sync + 'static,
>(
    socket_creator: impl Fn(SocketAddr) -> anyhow::Result<S> + Send + Sync + 'static,
    src: SocketAddr,
    orig_dst: Address<'static>,
    mut rx: Receiver<Vec<u8>>,
    upstream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    stats: Option<&UpstreamStatistics>,
    idling_duration: Duration,
) -> anyhow::Result<()> {
    let (mut upstream_r, mut upstream_w) = upstream.split();
    let (link_active_tx, mut link_active_rx) = bounded::<()>(10);
    let should_close_after_receive = orig_dst.get_port() == 53;

    // Upstream -> UDP
    let rx_count = stats.map(|v| v.rx.clone()).unwrap_or_default();
    let task1: Task<anyhow::Result<()>> = {
        let link_active_tx = link_active_tx.clone();
        spawn(async move {
            let mut packet_reader = PacketReader::new();
            let mut sockets = HashMap::<Address<'static>, _>::new();
            loop {
                let (buf, addr) = packet_reader.read(&mut upstream_r).await?;
                let socket_addr = match addr.resolve().await.next() {
                    Some(a) => a,
                    None => {
                        log::warn!("Unable to resolve {addr}");
                        continue;
                    }
                };

                sockets
                    .entry(addr.clone().into_owned())
                    .or_insert_with(|| socket_creator(socket_addr).unwrap())
                    .send_dgram(&buf, src)
                    .await?;

                let _ = link_active_tx.try_send(());
                rx_count.inc(buf.len());
            }
        })
    };

    // UDP -> upstream
    let tx_count = stats.map(|v| v.tx.clone()).unwrap_or_default();

    let task2: Task<anyhow::Result<()>> = spawn(async move {
        let mut writer = PacketWriter::new();
        while let Some(b) = rx.next().await {
            let _ = link_active_tx.try_send(());
            let written_len = writer.write(&mut upstream_w, &orig_dst, &b).await?;
            tx_count.inc(written_len);
            if should_close_after_receive {
                break;
            }
        }
        Ok(())
    });

    let mut task1 = task1.fuse();
    let mut task2 = task2.fuse();

    loop {
        select! {
            v1 = task1 => return v1,
            v2 = task2 => return v2,
            v3 = link_active_rx.next().timeout(idling_duration).fuse() => {
                if v3.is_none() {
                    bail!("Timeout")
                }
            }
        };
    }
}
