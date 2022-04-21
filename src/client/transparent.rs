use std::{
    borrow::Cow,
    collections::HashMap,
    io::ErrorKind,
    mem::{size_of, MaybeUninit},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    os::unix::prelude::{AsRawFd, FromRawFd},
    sync::Arc,
    time::Duration,
};

use crate::{
    config::ClientConfig,
    proxy::{
        protocol::ProxyRequest,
        udp::{PacketReader, PacketWriter},
    },
    rt::{
        mpsc::{bounded, Receiver, Sender, TrySendError},
        net::{resolve, UdpSocket},
        spawn, Task, TimeoutExt,
    },
    socks5::Address,
    utils::{new_vec_for_udp, VecExt},
};
use anyhow::{bail, Context};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, StreamExt};
use futures_util::{select, FutureExt};
use libc::{
    c_int, c_void, size_t, sockaddr_in, sockaddr_in6, socklen_t, ssize_t, AF_INET, AF_INET6,
    IPV6_RECVORIGDSTADDR, IP_RECVORIGDSTADDR, SOL_IP, SOL_IPV6,
};
use nix::sys::socket::{
    setsockopt, sockopt::IpTransparent, AddressFamily, InetAddr, SockAddr, SockFlag, SockProtocol,
};

use super::{utils::request_best_upstream, ClientStatistics, UpstreamStatistics};

struct UdpSession {
    tx: Sender<Vec<u8>>,
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

        let mut buf = new_vec_for_udp();
        loop {
            let ((len, src), dst) = select! {
                k = cleanup_rx.next().fuse() => {
                    let k = k.context("Timeout channel closed")?;
                    log::info!("UDP session {k:?} timeout");
                    sessions.remove(&k);
                    continue;
                }

                r = recv_with_orig_dst(&socket, &mut buf).fuse() => {
                    match r {
                        Ok(v) => v,
                        Err(e) => {
                            log::error!("Error receiving TProxy packet: {e:?}");
                            return Err(e.into());
                        }
                    }
                }
            };

            buf.set_len_uninit(len);

            log::debug!("TProxy received {len} from {src}, orig dst = {dst}");
            let key = UdpSessionKey { src, dst };
            buf = match sessions.get_mut(&key) {
                Some(s) => match s.tx.try_send(buf) {
                    Ok(_) => new_vec_for_udp(),
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
                    new_vec_for_udp()
                }
            };

            buf.set_len_to_capacity();
        }
    }))
}

fn buf_to_addr(buf: *const c_void, _buf_len: usize) -> Option<SocketAddr> {
    match unsafe { &*(buf as *const sockaddr_in) }.sin_family as c_int {
        AF_INET => {
            let addr = unsafe { &*(buf as *const sockaddr_in) };
            Some(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr)),
                u16::from_be(addr.sin_port),
            )))
        }
        AF_INET6 => {
            let addr = unsafe { &*(buf as *const sockaddr_in6) };
            Some(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(addr.sin6_addr.s6_addr),
                u16::from_be(addr.sin6_port),
                u32::from_be(addr.sin6_flowinfo),
                u32::from_be(addr.sin6_scope_id),
            )))
        }
        _ => None,
    }
}

extern "C" {
    fn do_recv_with_orig_dst(
        fd: c_int,
        buf: *mut c_void,
        buf_len: size_t,
        src_addr_buf: *mut c_void,
        src_addr_len: *mut socklen_t,
        dst_addr_buf: *mut c_void,
        dst_addr_len: *mut socklen_t,
    ) -> ssize_t;
}

async fn recv_with_orig_dst(
    socket: &UdpSocket,
    buf: &mut [u8],
) -> std::io::Result<((usize, SocketAddr), SocketAddr)> {
    socket
        .read_with(|socket| {
            const V6_ADDR_LEN: usize = size_of::<sockaddr_in6>();
            let mut src_addr: [MaybeUninit<u8>; V6_ADDR_LEN] =
                unsafe { MaybeUninit::uninit().assume_init() };

            let mut dst_addr: [MaybeUninit<u8>; V6_ADDR_LEN] =
                unsafe { MaybeUninit::uninit().assume_init() };

            let mut src_addr_len = src_addr.len() as socklen_t;
            let mut dst_addr_len = src_addr.len() as socklen_t;

            let rc = unsafe {
                do_recv_with_orig_dst(
                    socket.as_raw_fd() as c_int,
                    buf.as_mut_ptr() as *mut c_void,
                    buf.len() as size_t,
                    src_addr.as_mut_ptr() as *mut c_void,
                    &mut src_addr_len as &mut socklen_t,
                    dst_addr.as_mut_ptr() as *mut c_void,
                    &mut dst_addr_len as &mut socklen_t,
                )
            };

            if rc < 0 {
                return Err(std::io::Error::from_raw_os_error(nix::errno::errno()));
            }

            if src_addr_len == 0 || dst_addr_len == 0 {
                return Err(std::io::Error::new(
                    ErrorKind::AddrNotAvailable,
                    "SRC or DST addr unavailable",
                ));
            }

            let src = buf_to_addr(src_addr.as_ptr() as *const c_void, src_addr_len as usize)
                .ok_or_else(|| {
                    std::io::Error::new(ErrorKind::AddrNotAvailable, "SRC addr unavailable")
                })?;

            let dst = buf_to_addr(dst_addr.as_ptr() as *const c_void, dst_addr_len as usize)
                .ok_or_else(|| {
                    std::io::Error::new(ErrorKind::AddrNotAvailable, "DST addr unavailable")
                })?;

            Ok(((rc as usize, src), dst))
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
        initial_data: Vec<u8>,
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
            let mut sockets: HashMap<Address<'static>, UdpSocket> = Default::default();
            loop {
                let (buf, addr) = packet_reader.read(&mut upstream_r).await?;
                match sockets.get(&addr) {
                    Some(socket) => {
                        socket.send_to(&buf, src).await?;
                    }
                    None => {
                        let socket = bind_trasnparent_udp(&addr).await?;
                        socket.send_to(&buf, src).await?;
                        sockets.insert(addr.clone().into_owned(), socket);
                    }
                };

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

    unsafe {
        let value = 1usize;
        if libc::setsockopt(
            socket,
            match &addr {
                SocketAddr::V4(_) => SOL_IP,
                SocketAddr::V6(_) => SOL_IPV6,
            },
            match &addr {
                SocketAddr::V4(_) => IP_RECVORIGDSTADDR,
                SocketAddr::V6(_) => IPV6_RECVORIGDSTADDR,
            },
            &value as *const usize as *const c_void,
            size_of::<usize>() as socklen_t,
        ) != 0
        {
            return Err(std::io::Error::from_raw_os_error(nix::errno::errno()))
                .context("Setting RECV_ORIG_DST_ADDR");
        }
    }

    nix::sys::socket::bind(socket, &SockAddr::Inet(InetAddr::from_std(&addr)))
        .with_context(|| format!("Binding on {addr}"))?;

    log::debug!("UDP tproxy bound on {addr}");

    Ok(unsafe { std::net::UdpSocket::from_raw_fd(socket) }.try_into()?)
}

async fn serve_udp_direct(_rx: Receiver<Vec<u8>>) -> anyhow::Result<()> {
    bail!("Unimplemented")
}
