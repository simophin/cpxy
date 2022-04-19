use std::{
    borrow::Cow,
    collections::HashMap,
    io::ErrorKind,
    net::SocketAddr,
    os::unix::prelude::{AsRawFd, FromRawFd},
    ptr::null_mut,
    slice::from_raw_parts,
    sync::Arc,
    time::Duration,
};

use crate::{
    buf::Buf,
    config::ClientConfig,
    proxy::{
        protocol::ProxyRequest,
        udp::{Packet, PacketWriter},
    },
    rt::{
        mpsc::{bounded, Receiver, Sender, TrySendError},
        net::{resolve, UdpSocket},
        spawn, Task, TimeoutExt,
    },
    socks5::Address,
};
use anyhow::{bail, Context};
use futures_lite::{io::split, AsyncRead, AsyncWrite, StreamExt};
use futures_util::{select, FutureExt};
use libc::{
    c_void, iovec, msghdr, size_t, socklen_t, CMSG_DATA, CMSG_FIRSTHDR, CMSG_LEN, CMSG_NXTHDR,
    IPV6_RECVORIGDSTADDR, IP_RECVORIGDSTADDR, SOL_SOCKET,
};
use nix::{
    cmsg_space,
    sys::socket::{
        setsockopt, sockopt::IpTransparent, AddressFamily, InetAddr, SockAddr, SockFlag,
        SockProtocol,
    },
};

use super::{utils::request_best_upstream, ClientStatistics, UpstreamStatistics};

struct UdpSession {
    tx: Sender<Buf>,
    _task: Task<anyhow::Result<()>>,
}

#[derive(PartialEq, Eq, Hash, Debug)]
struct UdpSessionKey {
    src: SocketAddr,
    dst: SocketAddr,
}

pub async fn serve_udp_transparent_proxy(
    addr: SocketAddr,
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
) -> anyhow::Result<Task<anyhow::Result<()>>> {
    let socket = Arc::new(
        bind_trasnparent_udp(&addr.into())
            .await
            .context("Binding UDP socket")?,
    );
    log::info!("Started UDP transparent proxy at {addr}");
    Ok(spawn(async move {
        let mut sessions: HashMap<UdpSessionKey, UdpSession> = Default::default();
        let (cleanup_tx, mut cleanup_rx) = bounded::<UdpSessionKey>(2);

        let mut buf = Buf::new_for_udp();
        loop {
            let ((len, src), dst) = select! {
                k = cleanup_rx.next().fuse() => {
                    let k = k.context("Timeout channel closed")?;
                    log::info!("UDP session {k:?} timeout");
                    sessions.remove(&k);
                    continue;
                }

                r = recv_with_orig_dst(&socket, &mut buf).fuse() => {
                    r.context("Receiving tproxy proxy")?
                }
            };

            buf.set_len(len);

            log::debug!("TProxy received {len} from {src}, orig dst = {dst}");
            let key = UdpSessionKey { src, dst };
            buf = match sessions.get_mut(&key) {
                Some(s) => match s.tx.try_send(buf) {
                    Ok(_) => Buf::new_for_udp(),
                    Err(TrySendError::Closed(b)) => {
                        let key = UdpSessionKey { src, dst };
                        log::debug!("Session {key:?} closed");
                        sessions.remove(&key);
                        b
                    }
                    Err(TrySendError::Full(b)) => b,
                },
                None => {
                    let session = UdpSession::new(
                        src,
                        dst,
                        config.clone(),
                        stats.clone(),
                        cleanup_tx.clone(),
                        buf,
                    );
                    sessions.insert(key, session);
                    Buf::new_for_udp()
                }
            };

            buf.set_len(buf.capacity());
        }
    }))
}

fn buf_to_addr(b: &[u8]) -> std::io::Result<SocketAddr> {
    match b.len() {
        6 => todo!(),
        18 => todo!(),
        l => Err(std::io::Error::new(
            ErrorKind::Unsupported,
            format!("Wrong address len {l}: expecting 6 or 18"),
        )),
    }
}

async fn recv_with_orig_dst(
    socket: &UdpSocket,
    buf: &mut [u8],
) -> std::io::Result<((usize, SocketAddr), SocketAddr)> {
    socket
        .read_with(|socket| {
            let mut iov = iovec {
                iov_base: buf.as_mut_ptr() as *mut c_void,
                iov_len: buf.len(),
            };

            let mut cmsg_buf = cmsg_space!([u8; 18]);
            let mut src_addr = [0u8; 18];

            let mut hdr = msghdr {
                msg_name: src_addr.as_mut_ptr() as *mut c_void,
                msg_namelen: src_addr.len() as socklen_t,
                msg_iov: &mut iov as *mut iovec,
                msg_iovlen: 1,
                msg_control: cmsg_buf.as_mut_ptr() as *mut c_void,
                msg_controllen: cmsg_buf.len(),
                msg_flags: 0,
            };

            let rc = unsafe { libc::recvmsg(socket.as_raw_fd(), &mut hdr as *mut msghdr, 0) };

            if rc < 0 {
                return Err(std::io::Error::from_raw_os_error(nix::errno::errno()));
            }

            let mut orig_dst_addr = None;

            unsafe {
                let mut cmsg = CMSG_FIRSTHDR(&hdr);
                while cmsg != null_mut() {
                    let msg = *cmsg;
                    if msg.cmsg_level == SOL_SOCKET {
                        match (msg.cmsg_type, msg.cmsg_len) {
                            (IP_RECVORIGDSTADDR, len) if len == CMSG_LEN(6) as size_t => {
                                orig_dst_addr = Some(buf_to_addr(from_raw_parts(
                                    CMSG_DATA(cmsg) as *const u8,
                                    6,
                                ))?);
                                break;
                            }

                            (IPV6_RECVORIGDSTADDR, len) if len == CMSG_LEN(18) as size_t => {
                                orig_dst_addr = Some(buf_to_addr(from_raw_parts(
                                    CMSG_DATA(cmsg) as *const u8,
                                    18,
                                ))?);
                                break;
                            }

                            _ => {}
                        }
                    }

                    cmsg = CMSG_NXTHDR(&hdr, cmsg);
                }
            }

            Ok((
                (
                    rc as usize,
                    buf_to_addr(unsafe {
                        from_raw_parts(hdr.msg_name as *const u8, hdr.msg_namelen as usize)
                    })?,
                ),
                orig_dst_addr.ok_or_else(|| {
                    std::io::Error::new(ErrorKind::AddrNotAvailable, "OrigDst not available")
                })?,
            ))
        })
        .await
}

impl UdpSession {
    pub fn new(
        src: SocketAddr,
        dst: SocketAddr,
        config: Arc<ClientConfig>,
        stats: Arc<ClientStatistics>,
        clean_up: Sender<UdpSessionKey>,
        initial_data: Buf,
    ) -> Self {
        let (tx, rx) = bounded(10);
        let dst_addr: Address = dst.into();
        let _task = spawn(async move {
            let upstream = match request_best_upstream(
                &config,
                &stats,
                &dst_addr,
                &ProxyRequest::UDP {
                    initial_dst: dst_addr.clone(),
                    initial_data: Cow::Borrowed(&initial_data),
                },
            )
            .await
            {
                Ok((_, stream, stats)) => Some((stream, stats)),
                Err(err) => {
                    log::error!("Error requesting upstream: {err:?}");
                    None
                }
            };

            let result = if let Some((upstream, stats)) = upstream {
                // Proxy through upstream
                copy_between_udp_and_upstream(
                    src,
                    dst_addr,
                    rx,
                    upstream,
                    stats,
                    Duration::from_secs(60),
                )
                .await
            } else if config.allow_direct(&dst_addr) {
                // Direct connect
                serve_udp_direct(rx).await
            } else {
                Ok(())
            };

            clean_up.send(UdpSessionKey { src, dst }).await?;
            result
        });

        Self { tx, _task }
    }
}

async fn copy_between_udp_and_upstream(
    src: SocketAddr,
    orig_dst: Address<'static>,
    mut rx: Receiver<Buf>,
    upstream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    stats: Option<&UpstreamStatistics>,
    idling_duration: Duration,
) -> anyhow::Result<()> {
    let (upstream_r, mut upstream_w) = split(upstream);
    let (link_active_tx, mut link_active_rx) = bounded::<()>(10);

    // Upstream -> UDP
    let rx_count = stats.map(|v| v.rx.clone()).unwrap_or_default();
    let task1: Task<anyhow::Result<()>> = {
        let link_active_tx = link_active_tx.clone();
        spawn(async move {
            let mut packet_stream = Packet::new_packet_stream(upstream_r, None);
            let mut sockets: HashMap<Address<'static>, UdpSocket> = Default::default();
            while let Some((buf, addr)) = packet_stream.next().await {
                match sockets.get(&addr) {
                    Some(socket) => {
                        socket.send_to(buf.as_ref(), src).await?;
                    }
                    None => {
                        let socket = bind_trasnparent_udp(&addr).await?;
                        socket.send_to(buf.as_ref(), src).await?;
                        sockets.insert(addr, socket);
                    }
                };

                let _ = link_active_tx.try_send(());

                rx_count.inc(buf.as_ref().len());
            }

            Ok(())
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

async fn bind_trasnparent_udp(addr: &Address<'_>) -> anyhow::Result<UdpSocket> {
    let addr = match addr {
        Address::IP(addr) => *addr,
        Address::Name { host, port } => resolve((host.as_ref(), *port))
            .await?
            .into_iter()
            .next()
            .with_context(|| format!("Resolving {addr}"))?,
    };

    let socket = nix::sys::socket::socket(
        match &addr {
            SocketAddr::V4(_) => AddressFamily::Inet,
            SocketAddr::V6(_) => AddressFamily::Inet6,
        },
        nix::sys::socket::SockType::Datagram,
        SockFlag::SOCK_NONBLOCK,
        SockProtocol::Udp,
    )
    .context("Creating Unix Datagram socket")?;

    setsockopt(socket, IpTransparent, &true).context("Setting IP_TRANSPARNET on UdpSocket")?;
    // setsockopt(socket, OriginalDst, &true).context("Setting IP_TRANSPARNET on UdpSocket")?;
    nix::sys::socket::bind(socket, &SockAddr::Inet(InetAddr::from_std(&addr)))
        .with_context(|| format!("Binding on {addr}"))?;

    unsafe {
        let value: isize = 1;
        if libc::setsockopt(
            socket,
            SOL_SOCKET,
            match &addr {
                SocketAddr::V4(_) => IP_RECVORIGDSTADDR,
                SocketAddr::V6(_) => IPV6_RECVORIGDSTADDR,
            },
            &value as *const isize as *const c_void,
            std::mem::size_of::<isize>() as socklen_t,
        ) != 0
        {
            return Err(std::io::Error::from_raw_os_error(nix::errno::errno()).into());
        }
    }

    Ok(unsafe { std::net::UdpSocket::from_raw_fd(socket) }.try_into()?)
}

async fn serve_udp_direct(_rx: Receiver<Buf>) -> anyhow::Result<()> {
    todo!()
}
