use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::bail;
use futures_lite::{
    future::race, io::split, AsyncRead, AsyncReadExt, AsyncWrite, Stream, StreamExt,
};
use smol::{
    channel::{Receiver, Sender},
    spawn, Task,
};
use smol_timeout::TimeoutExt;

use crate::{
    buf::Buf,
    config::ClientConfig,
    handshake::Handshaker,
    io::UdpSocket,
    proxy::protocol::ProxyRequest,
    proxy::udp::write_packet_async as write_proxy_udp_packet_async,
    proxy::udp::Packet as ProxyUdpPacket,
    socks5::UdpPacket as Socks5UdpPacket,
    socks5::{Address, UdpRepr as Socks5UdpRepr},
    udp_relay::new_udp_relay,
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
    let (relay_addr, tx, rx) = match new_udp_relay(is_v4).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("Error creating UDP relay: {e:?}");
            handshaker.respond_err(&mut stream).await?;
            return Err(e);
        }
    };

    handshaker.respond_ok(&mut stream, Some(relay_addr)).await?;

    // Wait for first packet to decide where to go
    let pkt = rx.recv().await?;
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
            return race(
                copy_between_relay_and_stream(tx, rx, upstream, stat),
                drain_socks(stream),
            )
            .await;
        }
        Err(e) => {
            log::warn!("Error requesting best upstream for UDP://{addr}: {e:?}");
        }
    };

    if c.allow_direct(&addr) {
        return race(serve_udp_relay_directly(is_v4, tx, rx), drain_socks(stream)).await;
    }

    bail!("There's no where for UDP packet to go")
}

async fn serve_udp_relay_directly(
    v4: bool,
    tx: Sender<Socks5UdpPacket<Buf>>,
    rx: Receiver<Socks5UdpPacket<Buf>>,
) -> anyhow::Result<()> {
    let socket = Arc::new(UdpSocket::bind(v4).await?);
    let should_close_on_receive = Arc::new(AtomicBool::new(false));

    let task1: Task<anyhow::Result<()>> = {
        let socket = socket.clone();
        let should_close_on_receive = should_close_on_receive.clone();
        spawn(async move {
            while let Some(Ok(pkt)) = rx.recv().timeout(UDP_IDLING_TIMEOUT).await {
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
                socket.send_to_addr(pkt.payload(), &pkt_addr).await?;
            }

            Ok(())
        })
    };

    let task2: Task<anyhow::Result<()>> = spawn(async move {
        loop {
            let mut buf = Buf::new_for_udp();
            let (len, from) = socket.recv_from(&mut buf).await?;
            buf.set_len(len);

            log::debug!("UDP -> relay: Packet(len={len}, from={from})",);

            tx.send(
                Socks5UdpRepr {
                    addr: from.into(),
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

    let result = race(task1, task2).await;
    log::debug!("Ended serving UDP relay directly, result: {result:?}");
    result
}

async fn copy_between_relay_and_stream(
    tx: Sender<Socks5UdpPacket<Buf>>,
    mut rx: impl Stream<Item = Socks5UdpPacket<Buf>> + Unpin + Send + Sync + 'static,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    stats: Option<&UpstreamStatistics>,
) -> anyhow::Result<()> {
    let (mut stream_r, mut stream_w) = split(stream);

    // Proxy -> Relay
    let rx_count = stats.map(|s| s.rx.clone()).unwrap_or_default();
    let task1: Task<anyhow::Result<()>> = spawn(async move {
        let mut last_addr = None;

        loop {
            let pkt = ProxyUdpPacket::read_async(&mut stream_r).await?;
            log::debug!("Received {pkt:?} from upstream");
            let addr = match pkt
                .addr()
                .map(|a| a.into_owned())
                .or_else(|| last_addr.take())
            {
                Some(v) => v,
                None => {
                    log::info!("No source address available");
                    continue;
                }
            };

            rx_count.inc(pkt.inner().len());

            let repr = Socks5UdpRepr {
                addr,
                payload: pkt.payload(),
                frag_no: 0,
            };

            tx.send(repr.to_packet()?).await?;
            last_addr.replace(repr.addr);
        }
    });

    // Relay -> Proxy
    let tx_count = stats.map(|s| s.tx.clone()).unwrap_or_default();
    let task2: Task<anyhow::Result<()>> = spawn(async move {
        let mut last_sent_addr: Option<Address<'static>> = None;
        while let Some(Some(pkt)) = rx.next().timeout(UDP_IDLING_TIMEOUT).await {
            if pkt.frag_no() != 0 {
                log::warn!("Dropping fragmented socks5 packet");
                continue;
            }

            let send_addr = match (pkt.addr(), last_sent_addr.as_ref()) {
                (v, Some(last)) if last == &v => None,
                (v, _) => {
                    last_sent_addr.replace(v.into_owned());
                    last_sent_addr.as_ref()
                }
            };

            tx_count
                .inc(write_proxy_udp_packet_async(&mut stream_w, send_addr, pkt.payload()).await?);

            log::debug!("Sent {pkt:?} to upstream");
        }
        Ok(())
    });

    let result = race(task1, task2).await;
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

    use smol::{block_on, channel::bounded};
    use smol_timeout::TimeoutExt;

    use crate::test::duplex;

    use super::*;

    #[test]
    fn copy_relay_and_proxy_works() {
        block_on(async move {
            let (relay_in_tx, relay_in_rx) = bounded(2);
            let (relay_out_tx, relay_out_rx) = bounded(2);

            let (near_stream, far_stream) = duplex(10).await;

            let (mut far_stream_r, mut far_stream_w) = split(far_stream);

            let _task = spawn(copy_between_relay_and_stream(
                relay_out_tx,
                relay_in_rx,
                near_stream,
                None,
            ));

            struct PacketData<'a> {
                payload: &'a [u8],
                dst: &'a str,
                expect_stream_receives_dst: bool,
                reply_payload: &'a [u8],
                reply_src_addr: Option<&'a str>,
                expect_socks5_src_addr: &'a str,
            }

            let packets = [
                PacketData {
                    payload: b"hello, world1",
                    dst: "1.2.3.4:9000",
                    expect_stream_receives_dst: true,
                    reply_payload: b"reply1",
                    reply_src_addr: Some("5.5.5.5:2000"),
                    expect_socks5_src_addr: "5.5.5.5:2000",
                },
                PacketData {
                    payload: b"hello, world2",
                    dst: "1.2.3.4:9001",
                    expect_stream_receives_dst: true,
                    reply_payload: b"reply2",
                    reply_src_addr: None,
                    expect_socks5_src_addr: "5.5.5.5:2000",
                },
                PacketData {
                    payload: b"hello, world3",
                    dst: "1.2.3.4:9000",
                    expect_stream_receives_dst: true,
                    reply_payload: b"reply3",
                    reply_src_addr: Some("5.5.5.5:2001"),
                    expect_socks5_src_addr: "5.5.5.5:2001",
                },
                PacketData {
                    payload: b"hello, world4",
                    dst: "1.2.3.4:9000",
                    expect_stream_receives_dst: false,
                    reply_payload: b"reply4",
                    reply_src_addr: None,
                    expect_socks5_src_addr: "5.5.5.5:2001",
                },
                PacketData {
                    payload: b"hello, world4",
                    dst: "google.com:9000",
                    expect_stream_receives_dst: true,
                    reply_payload: b"reply5",
                    reply_src_addr: Some("5.5.5.5:2000"),
                    expect_socks5_src_addr: "5.5.5.5:2000",
                },
            ];

            for PacketData {
                payload,
                dst,
                expect_stream_receives_dst,
                reply_payload,
                reply_src_addr,
                expect_socks5_src_addr,
            } in packets
            {
                let dst: Address = dst.try_into().unwrap();
                relay_in_tx
                    .send(
                        Socks5UdpRepr {
                            addr: dst.clone(),
                            payload,
                            frag_no: 0,
                        }
                        .to_packet()
                        .unwrap(),
                    )
                    .await
                    .unwrap();

                let received = ProxyUdpPacket::read_async(&mut far_stream_r)
                    .timeout(Duration::from_secs(1))
                    .await
                    .unwrap()
                    .unwrap();

                // Test received stream packet
                assert_eq!(
                    received.addr().as_ref(),
                    if expect_stream_receives_dst {
                        Some(&dst)
                    } else {
                        None
                    }
                );
                assert_eq!(received.payload(), payload);

                // Reply...
                write_proxy_udp_packet_async(
                    &mut far_stream_w,
                    reply_src_addr.map(|v| v.try_into().unwrap()).as_ref(),
                    reply_payload,
                )
                .await
                .unwrap();

                // Test reply
                let pkt = relay_out_rx
                    .recv()
                    .timeout(Duration::from_secs(1))
                    .await
                    .unwrap()
                    .unwrap();

                assert_eq!(
                    pkt.addr(),
                    Address::try_from(expect_socks5_src_addr).unwrap()
                );
                assert_eq!(pkt.payload(), reply_payload);
            }
        });
    }

    #[test]
    fn serve_directly_works() {
        block_on(async move {
            let server = UdpSocket::bind(true).await.unwrap();
            let dst = server.local_addr().unwrap();
            let (relay_in_tx, relay_in_rx) = bounded(2);
            let (relay_out_tx, relay_out_rx) = bounded(2);

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
                            addr: dst.clone().into(),
                            payload,
                            frag_no: 0,
                        }
                        .to_packet()
                        .unwrap(),
                    )
                    .await
                    .unwrap();

                let mut buf = Buf::new_for_udp();
                let (len, addr) = server.recv_from(&mut buf).await.unwrap();
                buf.set_len(len);

                assert_eq!(buf.as_slice(), payload);

                // Reply
                server.send_to(reply_payload, addr).await.unwrap();

                // Check reply
                let pkt = relay_out_rx
                    .recv()
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
