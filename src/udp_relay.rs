use std::{net::SocketAddr, sync::Arc};

use anyhow::{anyhow, Context};
use bytes::Bytes;
use futures::{future::ready, Sink, SinkExt, Stream, StreamExt};
use parking_lot::Mutex;

use crate::{
    io::{bind_udp, UdpSocketExt},
    socks5::UdpPacket,
};

pub async fn new_udp_relay(
    v4: bool,
) -> anyhow::Result<(
    SocketAddr,
    impl Sink<UdpPacket<Bytes>, Error = anyhow::Error> + Unpin,
    impl Stream<Item = anyhow::Result<UdpPacket<Bytes>>> + Unpin,
)> {
    let udp = bind_udp(v4).await?;
    let bound_addr = udp.local_addr().context("Getting local_addr")?;

    let (sink, stream) = udp.to_sink_stream().split();

    let last_addr = Arc::new(Mutex::new(None));
    let sink = {
        let last_addr = last_addr.clone();
        sink.with(move |pkt: UdpPacket<Bytes>| {
            ready(
                last_addr
                    .lock()
                    .ok_or_else(|| anyhow!("No last address"))
                    .map(|addr| (pkt.into_inner(), addr)),
            )
        })
    };

    let stream = stream.filter_map(move |item| {
        let (data, addr) = match item {
            Ok(v) => v,
            Err(e) => return ready(Some(Err(e))),
        };

        last_addr.lock().replace(addr);
        let pkt = match UdpPacket::new_checked(data) {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error parsing SOCKS5 Udp: {e:?}");
                return ready(None);
            }
        };
        ready(Some(Ok(pkt)))
    });

    Ok((bound_addr, Box::pin(sink), Box::pin(stream)))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::StreamExt;

    use crate::rt::{block_on, TimeoutExt};

    use crate::socks5::{Address, UdpRepr};
    use crate::test::set_ip_local;
    use crate::utils::{new_vec_for_udp, VecExt};

    use super::*;

    #[test]
    fn udp_relay_works() -> anyhow::Result<()> {
        block_on(async move {
            let (mut relay_addr, mut tx, mut rx) = new_udp_relay(true).await?;
            set_ip_local(&mut relay_addr);

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
                .timeout(Duration::from_secs(2))
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
