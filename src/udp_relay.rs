use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context};
use bytes::Bytes;
use futures::StreamExt;

use crate::{
    io::bind_udp,
    rt::mpsc::{bounded, Receiver, Sender},
    rt::{spawn, Task},
    socks5::UdpPacket,
    utils::{new_vec_for_udp, VecExt},
};

pub async fn new_udp_relay(
    v4: bool,
) -> anyhow::Result<(
    SocketAddr,
    Sender<UdpPacket<Bytes>>,
    Receiver<UdpPacket<Bytes>>,
)> {
    let udp = Arc::new(bind_udp(v4).await?);
    let bound_addr = udp.local_addr().context("Getting local_addr")?;

    let (incoming_tx, incoming_rx) = bounded::<UdpPacket<Bytes>>(1);
    let (outgoing_tx, mut outgoing_rx) = bounded::<UdpPacket<Bytes>>(5);

    let last_addr: Arc<Mutex<Option<SocketAddr>>> = Default::default();

    {
        let udp = udp.clone();
        let last_addr = last_addr.clone();
        let task: Task<anyhow::Result<()>> = spawn(async move {
            loop {
                let pkt = outgoing_rx.next().await.context("Waiting for packet")?;
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
                let mut buf = new_vec_for_udp();
                let (len, src) = udp.recv_from(&mut buf).await?;
                buf.set_len_uninit(len);

                last_addr
                    .lock()
                    .map_err(|_| anyhow!("Unable to lock last_addr"))?
                    .replace(src);

                incoming_tx
                    .send(UdpPacket::new_checked(buf.into())?)
                    .await?;
            }
        });
        task.detach();
    }

    Ok((bound_addr, outgoing_tx, incoming_rx))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::StreamExt;

    use crate::rt::{block_on, TimeoutExt};

    use crate::socks5::{Address, UdpRepr};

    use super::*;

    #[test]
    fn udp_relay_works() -> anyhow::Result<()> {
        block_on(async move {
            let (relay_addr, tx, mut rx) = new_udp_relay(true).await?;

            let client = bind_udp(true).await?;

            let target_addr: Address = "google.com:600".parse()?;
            let payload = b"hello, world";

            // Sending
            client
                .send_to(
                    UdpRepr {
                        addr: &target_addr,
                        payload,
                        frag_no: 0,
                    }
                    .to_packet()?
                    .into_inner()
                    .as_ref(),
                    relay_addr,
                )
                .await?;

            let pkt = rx
                .next()
                .timeout(Duration::from_secs(1))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(pkt.addr(), target_addr);
            assert_eq!(pkt.payload(), payload);

            // Receiving
            let reply = b"hello, again!".as_ref();
            tx.send(
                UdpRepr {
                    addr: &target_addr,
                    payload: reply,
                    frag_no: 0,
                }
                .to_packet()?,
            )
            .await?;

            let mut buf = new_vec_for_udp();
            let (len, _) = client
                .recv_from(&mut buf)
                .timeout(Duration::from_secs(1))
                .await
                .unwrap()?;
            buf.set_len_uninit(len);

            let pkt = UdpPacket::new_checked(buf)?;
            assert_eq!(pkt.addr(), target_addr);
            assert_eq!(pkt.payload(), reply);

            Ok(())
        })
    }
}
