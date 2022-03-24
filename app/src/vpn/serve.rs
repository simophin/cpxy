use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
    net::{SocketAddr, SocketAddrV4},
    sync::{Arc, Mutex},
    task::Waker,
    time::Duration,
};

use anyhow::{bail, Context};
use futures_lite::{io::split, AsyncReadExt, AsyncWriteExt};
use futures_util::future::{select, Either};
use smol::{
    channel::{bounded, Receiver, Sender, TryRecvError, TrySendError},
    spawn, Task,
};
use smol_timeout::TimeoutExt;
use smoltcp::{
    iface::{Interface, InterfaceBuilder, Routes, SocketHandle},
    phy::Medium,
    socket::{TcpSocket, TcpSocketBuffer, TcpState},
    time::Instant,
    wire::{
        IpAddress, IpCidr, IpEndpoint, IpProtocol, IpVersion, Ipv4Address, Ipv4Packet, TcpPacket,
    },
};

use crate::buf::{Buf, RWBuffer};

use super::{channel::ChannelDevice, tcp::AsyncTcpSocket};

pub async fn serve() -> anyhow::Result<Task<anyhow::Result<()>>> {
    let mtu = 1500usize;
    let (device, tx, rx) = ChannelDevice::new(mtu, Medium::Ip);
    let (mut tun_dev_r, mut tun_dev_w) = split(super::tun::Device::new(
        "192.168.30.1".parse()?,
        "255.255.255.0".parse()?,
        mtu,
    )?);

    let mut routes = Routes::new(BTreeMap::new());
    routes.add_default_ipv4_route(Ipv4Address::new(192, 168, 30, 2))?;

    let mut iface = InterfaceBuilder::new(device, vec![])
        .any_ip(true)
        .routes(routes)
        .ip_addrs(vec![IpCidr::new(
            IpAddress::Ipv4(Ipv4Address::new(192, 168, 30, 2)),
            24,
        )])
        .finalize();

    let (poll_tx, poll_rx) = bounded::<()>(1);
    let mut tcp_connections = HashMap::<ClientKey, TcpClient>::new();
    let (tcp_socket_tx, tcp_socket_rx) = bounded::<AsyncTcpSocket>(10);
    let mut timeout = Duration::from_secs(1);
    let mut buf = Buf::new_with_len(mtu, mtu);

    Ok(spawn(async move {
        let _write_task = {
            let poll_tx = poll_tx.clone();
            spawn(async move {
                while let Ok(b) = rx.recv().await {
                    if let Err(e) = tun_dev_w.write(&b).await {
                        log::error!("Error writing to tun device: {e:?}");
                    }

                    // Notify poller
                    let _ = poll_tx.try_send(());
                }
            })
        };

        let _handle_task = spawn(async move {
            if let Err(e) = handle_tcp_sockets(tcp_socket_rx, mtu).await {
                log::error!("Error handling tcp sockets: {e:?}");
            }
        });

        loop {
            match select(tun_dev_r.read(&mut buf), poll_rx.recv())
                .timeout(timeout)
                .await
            {
                Some(Either::Left((len, _))) => {
                    let len = len.context("Reading TUN device")?;
                    buf.set_len(len);

                    if let Err(e) = handle_tun_recv(
                        &buf,
                        &mut iface,
                        &mut tcp_connections,
                        &poll_tx,
                        &tcp_socket_tx,
                    )
                    .await
                    {
                        log::error!("Error handling packet: {e:?}");
                    }

                    buf = match tx.try_send(buf) {
                        Ok(_) => Buf::new_with_len(mtu, mtu),
                        Err(TrySendError::Full(mut b) | TrySendError::Closed(mut b)) => {
                            log::info!("Unable to send tun packet to iface");
                            b.set_len(b.capacity());
                            b
                        }
                    }
                }
                _ => {}
            }

            let now = Instant::now();

            if let Err(e) = iface.poll(now) {
                log::warn!("Error while polling interface: {e:?}");
            }

            if let Err(e) = tick(&mut iface, &mut tcp_connections).await {
                log::error!("Error ticking: {e:?}");
            }

            timeout = match iface.poll_delay(now) {
                Some(t) => t.into(),
                None => Duration::from_secs(5),
            };

            log::debug!("Next polling at {timeout:?} later");
        }
    }))
}

async fn tick<'a>(
    iface: &mut Interface<'a, ChannelDevice>,
    tcp_connections: &mut HashMap<ClientKey, TcpClient>,
) -> anyhow::Result<()> {
    tcp_connections.retain(|key, handle| {
        let socket: &mut TcpSocket = iface.get_socket(handle.handle);

        if socket.may_recv() {
            let mut rx = handle.rx.lock().expect("To lock rx");

            if rx.0.remaining_write() > 0 {
                let _ = socket.recv(|buf| {
                    let len = rx.0.remaining_write().min(buf.len());
                    let _ = rx.0.write(&buf[..len]);
                    rx.0.advance_write(len);

                    (len, ())
                });
            }

            if let Some(waker) = rx.1.take() {
                socket.register_recv_waker(&waker);
            }
        }

        if socket.may_send() {
            let mut tx = handle.tx.lock().expect("To lock tx");

            if tx.0.remaining_read() > 0 {
                let _ = socket.send(|out| {
                    let len = tx.0.remaining_read().min(out.len());
                    log::debug!("Sent {} data on tcp", len);
                    (&mut out[..len]).copy_from_slice(&tx.0.read_buf()[..len]);
                    tx.0.advance_read(len);
                    (len, ())
                });
            }

            if let Some(waker) = tx.1.take() {
                socket.register_send_waker(&waker);
            }
        }

        let close_waker = handle
            .close_requested
            .lock()
            .expect("To lock close handle")
            .take();

        if close_waker.is_some() || socket.state() == TcpState::CloseWait {
            socket.close()
        }

        if let Some(waker) = close_waker {
            waker.wake();
        }

        if socket.is_open() {
            true
        } else {
            iface.remove_socket(handle.handle);
            log::debug!("Removing client {key:?}");
            false
        }
    });
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct ClientKey {
    local: SocketAddr,
    remote: SocketAddr,
}

struct TcpClient {
    handle: SocketHandle,
    tx: Arc<Mutex<(RWBuffer, Option<Waker>)>>,
    rx: Arc<Mutex<(RWBuffer, Option<Waker>)>>,
    close_requested: Arc<Mutex<Option<Waker>>>,
}

async fn handle_tun_recv<'a>(
    buf: &Buf,
    iface: &mut Interface<'a, ChannelDevice>,
    tcp_connections: &mut HashMap<ClientKey, TcpClient>,
    poll_tx: &Sender<()>,
    socket_tx: &Sender<AsyncTcpSocket>,
) -> anyhow::Result<()> {
    if IpVersion::of_packet(&buf) != Ok(IpVersion::Ipv4) {
        log::debug!("Skip non-ipv4 packet");
        return Ok(());
    }

    // Parse IP packet
    let ip = match Ipv4Packet::new_checked(buf.as_ref()) {
        Ok(v) => v,
        Err(e) => {
            bail!("Received invalid IP pkt: {e:?}");
        }
    };

    // Parse tcp packet
    let tcp = match ip.protocol() {
        IpProtocol::Tcp => match TcpPacket::new_checked(ip.payload()) {
            Ok(v) => Some(v),
            Err(e) => {
                bail!("Error parsing TCP packet: {e:?}");
            }
        },
        v => {
            log::info!("Skipping IP packet protocol {v}");
            None
        }
    };

    match tcp {
        Some(tcp) if tcp.syn() => {
            let remote = SocketAddr::V4(SocketAddrV4::new(ip.dst_addr().into(), tcp.dst_port()));
            let local = SocketAddr::V4(SocketAddrV4::new(ip.src_addr().into(), tcp.src_port()));
            let key = ClientKey { local, remote };

            if tcp_connections.contains_key(&key) {
                log::warn!("Connection {key:?} already established");
                return Ok(());
            }

            log::info!("Received new connection from {local} to {remote}");
            let mtu = iface.device().mtu;

            let handle = iface.add_socket(TcpSocket::new(
                TcpSocketBuffer::new(vec![0u8; mtu]),
                TcpSocketBuffer::new(vec![0u8; mtu]),
            ));

            iface
                .get_socket::<TcpSocket>(handle)
                .listen(IpEndpoint::new(
                    IpAddress::Ipv4(ip.dst_addr()),
                    tcp.dst_port(),
                ))
                .with_context(|| format!("Listening on {remote:?}"))?;

            let poll_tx = poll_tx.clone();
            let tx = Arc::new(Mutex::new((RWBuffer::new(mtu, mtu), None)));
            let rx = Arc::new(Mutex::new((RWBuffer::new(mtu, mtu), None)));
            let close_requested = Arc::new(Mutex::new(None));
            let asocket =
                AsyncTcpSocket::new(local, remote, &tx, &rx, close_requested.clone(), poll_tx);
            tcp_connections.insert(
                key,
                TcpClient {
                    handle,
                    tx,
                    rx,
                    close_requested,
                },
            );
            let _ = socket_tx.send(asocket).await; // Notify a new socket
        }
        _ => {}
    }

    Ok(())
}

async fn handle_tcp_sockets(rx: Receiver<AsyncTcpSocket>, mtu: usize) -> anyhow::Result<()> {
    loop {
        let mut socket = rx.recv().await?;
        log::info!("Handling Socket from {} to {}", socket.src, socket.dst);
        
    }
}
