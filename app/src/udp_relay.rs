use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context};
use smol::{
    channel::{bounded, Receiver, Sender},
    spawn, Task,
};

use crate::{buf::Buf, io::UdpSocket, socks5::UdpPacket};

pub async fn new_udp_relay(
    v4: bool,
) -> anyhow::Result<(SocketAddr, Sender<UdpPacket<Buf>>, Receiver<UdpPacket<Buf>>)> {
    let udp = Arc::new(UdpSocket::bind(v4).await?);
    let bound_addr = udp.local_addr().context("Getting local_addr")?;

    let (incoming_tx, incoming_rx) = bounded::<UdpPacket<Buf>>(1);
    let (outgoing_tx, outgoing_rx) = bounded::<UdpPacket<Buf>>(5);

    let last_addr: Arc<Mutex<Option<SocketAddr>>> = Default::default();

    {
        let udp = udp.clone();
        let last_addr = last_addr.clone();
        let task: Task<anyhow::Result<()>> = spawn(async move {
            loop {
                let pkt = outgoing_rx.recv().await?;
                let dst = last_addr
                    .lock()
                    .map_err(|_| anyhow!("Unable to lock last_addr"))?
                    .as_ref()
                    .map(|a| a.clone());

                if let Some(dst) = dst {
                    udp.send_to(pkt.inner().as_ref(), dst).await?;
                }
            }
        });
        task.detach();
    }

    {
        let task: Task<anyhow::Result<()>> = spawn(async move {
            loop {
                let mut buf = Buf::new_for_udp();
                let (len, src) = udp.recv_from(&mut buf).await?;
                buf.set_len(len);

                last_addr
                    .lock()
                    .map_err(|_| anyhow!("Unable to lock last_addr"))?
                    .replace(src);

                incoming_tx.send(UdpPacket::new_checked(buf)?).await?;
            }
        });
        task.detach();
    }

    Ok((bound_addr, outgoing_tx, incoming_rx))
}
