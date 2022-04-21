use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::{
    io::{bind_udp, send_to_addr},
    rt::{
        mpsc::{Receiver, Sender},
        spawn, Task, TimeoutExt,
    },
    utils::{new_vec_for_udp, race, VecExt},
};
use anyhow::{bail, Context};
use bytes::Bytes;
use futures::{select, AsyncRead, AsyncReadExt, AsyncWrite, FutureExt, StreamExt};

use crate::{
    config::ClientConfig, handshake::Handshaker, proxy::protocol::ProxyRequest,
    proxy::udp::PacketReader as ProxyUdpPacketReader,
    proxy::udp::PacketWriter as ProxyUdpPacketWriter, socks5::UdpPacket as Socks5UdpPacket,
    socks5::UdpRepr as Socks5UdpRepr, udp_relay::new_udp_relay,
};

use super::{utils::request_best_upstream, ClientStatistics, UpstreamStatistics};

const UDP_IDLING_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn serve_udp_proxy_conn(
    c: &ClientConfig,
    stats: &ClientStatistics,
    is_v4: bool,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let (relay_addr, tx, mut rx) = match new_udp_relay(is_v4).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("Error creating UDP relay: {e:?}");
            handshaker.respond_err(&mut stream).await?;
            return Err(e);
        }
    };

    handshaker.respond_ok(&mut stream, Some(relay_addr)).await?;

    // Wait for first packet to decide where to go
    let pkt = rx.next().await.context("Waiting for first packet")?;
    let addr = pkt.addr().into_owned();

    match request_best_upstream(
        c,
        stats,
        &addr,
        &ProxyRequest::UDP {
            initial_dst: pkt.addr(),
            initial_data: Cow::Borrowed(pkt.payload()),
        },
    )
    .await
    {
        Ok((_, upstream, stat)) => {
            return select! {
                r1 = copy_between_relay_and_stream(tx, rx, upstream, stat).fuse() => r1,
                r2 = drain_socks(stream).fuse() => r2,
            };
        }
        Err(e) => {
            log::warn!("Error requesting best upstream for UDP://{addr}: {e:?}");
        }
    };

    if c.allow_direct(&addr) {
        return select! {
            r1 = serve_udp_relay_directly(is_v4, tx, rx).fuse() => r1,
            r2 = drain_socks(stream).fuse() => r2,
        };
    }

    bail!("There's no where for UDP packet to go")
}

async fn serve_udp_relay_directly(
    v4: bool,
    tx: Sender<Socks5UdpPacket<Bytes>>,
    mut rx: Receiver<Socks5UdpPacket<Bytes>>,
) -> anyhow::Result<()> {
    let socket = Arc::new(bind_udp(v4).await?);
    let should_close_on_receive = Arc::new(AtomicBool::new(false));

    let task1: Task<anyhow::Result<()>> = {
        let socket = socket.clone();
        let should_close_on_receive = should_close_on_receive.clone();
        spawn(async move {
            while let Some(Some(pkt)) = rx.next().timeout(UDP_IDLING_TIMEOUT).await {
                if pkt.frag_no() != 0 {
                    log::warn!("Dropping fragmented packet");
                    continue;
                }

                let pkt_addr = pkt.addr();

                // Close DNS request immediately after receiving the response
                if pkt_addr.get_port() == 53 {
                    should_close_on_receive.store(true, Ordering::SeqCst);
                }

                log::debug!(
                    "Relay -> UDP: Packet(len={}, dst={pkt_addr})",
                    pkt.payload().len()
                );
                send_to_addr(&socket, pkt.payload(), &pkt_addr).await?;
            }

            Ok(())
        })
    };

    let task2: Task<anyhow::Result<()>> = spawn(async move {
        loop {
            let mut buf = new_vec_for_udp();
            let (len, from) = socket.recv_from(&mut buf).await?;
            buf.set_len_uninit(len);

            log::debug!("UDP -> relay: Packet(len={len}, from={from})",);

            tx.send(
                Socks5UdpRepr {
                    addr: &from.into(),
                    payload: buf,
                    frag_no: 0,
                }
                .to_packet()?,
            )
            .await?;

            if should_close_on_receive.load(Ordering::Relaxed) {
                break Ok(());
            }
        }
    });

    let result = race(task1.fuse(), task2.fuse()).await;
    log::debug!("Ended serving UDP relay directly, result: {result:?}");
    result
}

async fn copy_between_relay_and_stream(
    tx: Sender<Socks5UdpPacket<Bytes>>,
    mut rx: Receiver<Socks5UdpPacket<Bytes>>,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    stats: Option<&UpstreamStatistics>,
) -> anyhow::Result<()> {
    let (mut stream_r, mut stream_w) = stream.split();

    // Proxy -> Relay
    let rx_count = stats.map(|s| s.rx.clone()).unwrap_or_default();
    let task1: Task<anyhow::Result<()>> = spawn(async move {
        let mut reader = ProxyUdpPacketReader::new();

        loop {
            let (buf, addr) = reader.read(&mut stream_r).await?;
            rx_count.inc(buf.as_ref().len());

            let repr = Socks5UdpRepr {
                addr,
                payload: buf.as_ref(),
                frag_no: 0,
            };

            tx.send(repr.to_packet()?).await?;
        }
    });

    // Relay -> Proxy
    let tx_count = stats.map(|s| s.tx.clone()).unwrap_or_default();
    let task2: Task<anyhow::Result<()>> = spawn(async move {
        let mut writer = ProxyUdpPacketWriter::new();
        while let Some(Some(pkt)) = rx.next().timeout(UDP_IDLING_TIMEOUT).await {
            if pkt.frag_no() != 0 {
                log::warn!("Dropping fragmented socks5 packet");
                continue;
            }

            tx_count.inc(
                writer
                    .write(&mut stream_w, &pkt.addr(), pkt.payload())
                    .await?,
            );

            log::debug!("Sent {pkt:?} to upstream");
        }
        Ok(())
    });

    let result = race(task1.fuse(), task2.fuse()).await;
    log::debug!("Ended serving UDP relay through TCP, result: {result:?}");
    result
}

async fn drain_socks(
    mut socks: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 24];
    while socks.read(buf.as_mut_slice()).await? > 0 {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::rt::{block_on, mpsc::bounded, TimeoutExt};
    use crate::socks5::Address;
    use crate::test::duplex;
    use crate::utils::VecExt;

    use super::*;

    #[test]
    fn copy_relay_and_proxy_works() {
        block_on(async move {
            let (relay_in_tx, relay_in_rx) = bounded(2);
            let (relay_out_tx, mut relay_out_rx) = bounded(2);

            let (near_stream, far_stream) = duplex(10).await;

            let (mut far_stream_r, mut far_stream_w) = far_stream.split();

            let _task = spawn(copy_between_relay_and_stream(
                relay_out_tx,
                relay_in_rx,
                near_stream,
                None,
            ));

            struct PacketData<'a> {
                payload: &'a [u8],
                dst: &'a str,
                reply_payload: &'a [u8],
                reply_src_addr: &'a str,
            }

            let packets = [
                PacketData {
                    payload: b"hello, world1",
                    dst: "1.2.3.4:9000",
                    reply_payload: b"reply1",
                    reply_src_addr: "5.5.5.5:2000",
                },
                PacketData {
                    payload: b"hello, world2",
                    dst: "1.2.3.4:9001",
                    reply_payload: b"reply2",
                    reply_src_addr: "5.5.5.5:2000",
                },
                PacketData {
                    payload: b"hello, world3",
                    dst: "1.2.3.4:9000",
                    reply_payload: b"reply3",
                    reply_src_addr: "5.5.5.5:2001",
                },
                PacketData {
                    payload: b"hello, world4",
                    dst: "1.2.3.4:9000",
                    reply_payload: b"reply4",
                    reply_src_addr: "5.5.5.5:2001",
                },
                PacketData {
                    payload: b"hello, world4",
                    dst: "google.com:9000",
                    reply_payload: b"reply5",
                    reply_src_addr: "5.5.5.5:2000",
                },
            ];

            let mut packet_reader = ProxyUdpPacketReader::new();
            let mut packet_writer = ProxyUdpPacketWriter::new();

            for PacketData {
                payload,
                dst,
                reply_payload,
                reply_src_addr,
            } in packets
            {
                let dst: Address = dst.try_into().unwrap();
                relay_in_tx
                    .send(
                        Socks5UdpRepr {
                            addr: &dst,
                            payload,
                            frag_no: 0,
                        }
                        .to_packet()
                        .unwrap(),
                    )
                    .await
                    .unwrap();

                let received = packet_reader
                    .read(&mut far_stream_r)
                    .timeout(Duration::from_secs(1))
                    .await
                    .unwrap()
                    .unwrap();

                // Test received stream packet
                assert_eq!(received.1, &dst);
                assert_eq!(received.0.as_ref(), payload);

                // Reply...
                let reply_src_addr: Address = reply_src_addr.try_into().unwrap();
                packet_writer
                    .write(&mut far_stream_w, &reply_src_addr, reply_payload)
                    .await
                    .unwrap();

                // Test reply
                let pkt = relay_out_rx
                    .next()
                    .timeout(Duration::from_secs(1))
                    .await
                    .unwrap()
                    .unwrap();

                assert_eq!(pkt.addr(), reply_src_addr);
                assert_eq!(pkt.payload(), reply_payload);
            }
        });
    }
    #[test]
    fn serve_directly_works() {
        block_on(async move {
            let server = bind_udp(true).await.unwrap();
            let dst = server.local_addr().unwrap();
            let (relay_in_tx, relay_in_rx) = bounded(2);
            let (relay_out_tx, mut relay_out_rx) = bounded(2);

            let _task = spawn(serve_udp_relay_directly(true, relay_out_tx, relay_in_rx));

            struct PacketData<'a> {
                payload: &'a [u8],
                reply_payload: &'a [u8],
            }

            let packets = [
                PacketData {
                    payload: b"payload1",
                    reply_payload: b"reply1",
                },
                PacketData {
                    payload: b"payload2",
                    reply_payload: b"reply2",
                },
                PacketData {
                    payload: b"payload3",
                    reply_payload: b"reply3",
                },
            ];

            for PacketData {
                payload,
                reply_payload,
            } in packets
            {
                relay_in_tx
                    .send(
                        Socks5UdpRepr {
                            addr: &dst.into(),
                            payload,
                            frag_no: 0,
                        }
                        .to_packet()
                        .unwrap(),
                    )
                    .await
                    .unwrap();

                let mut buf = new_vec_for_udp();
                let (len, addr) = server.recv_from(&mut buf).await.unwrap();
                buf.set_len_uninit(len);

                assert_eq!(buf.as_slice(), payload);

                // Reply
                server.send_to(reply_payload, addr).await.unwrap();

                // Check reply
                let pkt = relay_out_rx
                    .next()
                    .timeout(Duration::from_secs(1))
                    .await
                    .unwrap()
                    .unwrap();

                assert_eq!(
                    pkt.addr().get_port(),
                    Address::try_from(dst).unwrap().get_port()
                );
                assert_eq!(pkt.payload(), reply_payload);
            }
        });
    }
}
