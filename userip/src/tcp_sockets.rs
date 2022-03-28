use std::{collections::HashMap, net::SocketAddr, time::Duration};

use anyhow::bail;
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    AsyncRead, SinkExt, Stream, StreamExt,
};
use rand::Rng;
use smol::{spawn, Task, Timer};
use smoltcp::wire::{IpProtocol, TcpSeqNumber};

use crate::{
    buf::Pool,
    utils::{wait_for, WaitForResult},
    IpPacket,
};

use super::tcp::{ack, fin, TcpPacket};

#[derive(Debug)]
enum TcpState {
    SynReceived {
        payload_outgoing_tx: Sender<TcpPacket>,
        payload_incoming_rx: Receiver<TcpPacket>,
    },
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
    Closed,
}

pub struct TcpSocket {
    payload_outgoing_tx: SendHalf,
    payload_incoming_rx: ReadHalf,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct TcpSocketKey {
    src: SocketAddr,
    dst: SocketAddr,
}

struct TcpSocketHandle {
    incoming_tx: Sender<TcpPacket>,
    _task: Task<anyhow::Result<()>>,
}

struct TcpServerSocket {
    pool: Pool<Vec<u8>>,
    outgoing_tx: Sender<IpPacket>,
    sockets: HashMap<TcpSocketKey, TcpSocketHandle>,
    socket_sender: Sender<TcpSocket>,
    mtu: usize,
}

impl TcpServerSocket {
    pub fn new(
        pool: Pool<Vec<u8>>,
        tx: Sender<IpPacket>,
        mtu: usize,
    ) -> (Self, impl Stream<Item = TcpSocket> + Unpin + 'static) {
        let (socket_sender, socket_receiver) = channel(2);
        (
            Self {
                pool,
                outgoing_tx: tx,
                sockets: Default::default(),
                socket_sender,
                mtu,
            },
            socket_receiver,
        )
    }

    pub fn process(&mut self, ip: IpPacket) -> Result<(), IpPacket> {
        if ip.protocol() != IpProtocol::Tcp {
            return Err(ip);
        }

        let src_addr = ip.src_addr();
        let dst_addr = ip.dst_addr();
        let tcp = TcpPacket::new_checked(ip)?;
        let key = TcpSocketKey {
            src: SocketAddr::new(src_addr, tcp.src_port()),
            dst: SocketAddr::new(dst_addr, tcp.dst_port()),
        };

        match (tcp.syn(), self.sockets.get_mut(&key)) {
            (true, None) => {
                log::info!("Accepted socket key = {key:?}");
                let handle = start_serving_socket(
                    self.pool.clone(),
                    self.socket_sender.clone(),
                    self.outgoing_tx.clone(),
                    key.dst,
                    key.src,
                    tcp.seq_number(),
                    self.mtu,
                );

                self.sockets.insert(key, handle);
                Ok(())
            }
            (false, Some(handle)) => match handle.incoming_tx.try_send(tcp) {
                Err(e) if e.is_disconnected() => {
                    self.sockets.remove(&key);
                    Ok(())
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }
}

fn start_serving_socket(
    pool: Pool<Vec<u8>>,
    mut socket_sender: Sender<TcpSocket>,
    mut outgoing_ip_tx: Sender<IpPacket>,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
    mut remote_seq: TcpSeqNumber,
    mtu: usize,
) -> TcpSocketHandle {
    let (incoming_ip_tx, mut incoming_ip_rx) = channel::<TcpPacket>(10);
    let (payload_outgoing_tx, mut payload_outgoing_rx) = channel::<TcpPacket>(10);
    let (mut payload_incoming_tx, payload_incoming_rx) = channel::<TcpPacket>(10);
    let wait_timeout = Duration::from_secs(1);
    let _task: Task<anyhow::Result<()>> = spawn(async move {
        let mut state = TcpState::SynReceived {
            payload_incoming_rx,
            payload_outgoing_tx,
        };
        let mut local_seq = TcpSeqNumber(rand::thread_rng().gen());

        // Send SYN ACK
        outgoing_ip_tx
            .feed(ack(
                &pool,
                local_addr,
                remote_addr,
                local_seq,
                remote_seq + 1,
            ))
            .await?;

        // Set timer to wait for initial ACK
        let mut timer = Timer::after(wait_timeout);

        loop {
            state = match (
                wait_for(&mut incoming_ip_rx, &mut payload_outgoing_rx, &mut timer).await,
                state,
            ) {
                // ACK for SYN-ACK
                (
                    WaitForResult::Stream1(Some(tcp)),
                    TcpState::SynReceived {
                        payload_outgoing_tx,
                        payload_incoming_rx,
                    },
                ) if tcp.ack()
                    && tcp.seq_number() == remote_seq + 1
                    && tcp.ack_number() == local_seq + 1 =>
                {
                    local_seq += 1;
                    remote_seq += 1;
                    socket_sender
                        .feed(TcpSocket {
                            payload_outgoing_tx: SendHalf {
                                tx: payload_outgoing_tx,
                                pending: Default::default(),
                                ip_mtu: mtu,
                            },
                            payload_incoming_rx: ReadHalf {
                                rx: payload_incoming_rx,
                                ip_mtu: mtu,
                            },
                        })
                        .await?;
                    TcpState::Established
                }

                // ACK for SYN-ACK timeout
                (WaitForResult::Stream3(_), TcpState::SynReceived { .. }) => {
                    bail!("Timeout waiting for ACK for SYN-ACK");
                }

                // FIN
                (WaitForResult::Stream1(Some(tcp)), _)
                    if tcp.fin() && tcp.seq_number() == remote_seq + 1 =>
                {
                    log::debug!("Received FIN from {remote_addr}");
                    local_seq += 1;
                    remote_seq += 1;
                    let _ = outgoing_ip_tx
                        .feed(ack(&pool, local_addr, remote_addr, local_seq, remote_seq))
                        .await?;

                    local_seq += 1;
                    let _ = outgoing_ip_tx
                        .feed(fin(&pool, local_addr, remote_addr, local_seq))
                        .await?;
                    timer.set_after(wait_timeout);
                    TcpState::CloseWait
                }

                // FIN-ACK
                (WaitForResult::Stream1(Some(tcp)), TcpState::CloseWait)
                    if tcp.ack() && tcp.seq_number() == remote_seq + 1 =>
                {
                    log::debug!("Received FIN-ACK from {remote_addr}");
                    return Ok(());
                }

                // FIN-ACK timeout
                (WaitForResult::Stream3(_), TcpState::CloseWait) => {
                    bail!("Timeout waiting for FIN-ACK");
                }

                // IP protocol closed
                (WaitForResult::Stream1(None), _) => {
                    log::debug!("Upstream protocol closing");
                    return Ok(());
                }

                // Downstream closed
                (WaitForResult::Stream2(None), TcpState::Established) => {
                    local_seq += 1;
                    outgoing_ip_tx
                        .feed(fin(&pool, local_addr, remote_addr, local_seq))
                        .await?;
                    timer.set_after(wait_timeout);
                    TcpState::FinWait1
                }

                // ACK FIN received
                (WaitForResult::Stream1(Some(tcp)), TcpState::FinWait1)
                    if tcp.ack()
                        && tcp.seq_number() == remote_seq + 1
                        && tcp.ack_number() == local_seq + 1 =>
                {
                    local_seq += 1;
                    remote_seq += 1;
                    TcpState::FinWait2
                }

                // ACK FIN timeout
                (WaitForResult::Stream3(_), TcpState::FinWait1) => {
                    bail!("Timeout waiting for ACK_FIN");
                }

                // Final FIN
                (WaitForResult::Stream1(Some(tcp)), TcpState::FinWait2)
                    if tcp.fin()
                        && tcp.seq_number() == remote_seq + 1
                        && tcp.ack_number() == local_seq + 1 =>
                {
                    local_seq += 1;
                    remote_seq += 1;
                    outgoing_ip_tx
                        .feed(ack(&pool, local_addr, remote_addr, local_seq, remote_seq))
                        .await?;
                    return Ok(());
                }

                // Data
                (WaitForResult::Stream1(Some(tcp)), TcpState::Established) if !tcp.ack() => {
                    if tcp.seq_number() == remote_seq + 1 {
                        remote_seq += 1;
                        payload_incoming_tx.feed(tcp).await?;
                    }

                    TcpState::Established
                }

                (r, state) => {
                    log::warn!("Invalid packet(state = {state:?}) {r:?}");
                    state
                }
            }
        }
    });

    TcpSocketHandle {
        incoming_tx: incoming_ip_tx,
        _task,
    }
}

struct ReadHalf {
    rx: Receiver<TcpPacket>,
    ip_mtu: usize,
}
struct SendHalf {
    tx: Sender<TcpPacket>,
    pending: Vec<u8>,
    ip_mtu: usize,
}

impl AsyncRead for ReadHalf {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        todo!()
    }
}
