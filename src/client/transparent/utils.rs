use std::{
    collections::HashMap,
    io::ErrorKind,
    mem::{size_of, MaybeUninit},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    os::unix::prelude::{AsRawFd, FromRawFd, RawFd},
    pin::Pin,
    task::Poll,
};

use anyhow::Context;
use bytes::Bytes;
use futures::{ready, Sink, Stream};
use nix::sys::socket::{
    setsockopt, sockopt::IpTransparent, AddressFamily, SockFlag, SockProtocol, SockaddrIn,
    SockaddrIn6,
};

use libc::{
    c_int, c_void, size_t, sockaddr_in, sockaddr_in6, socklen_t, ssize_t, AF_INET, AF_INET6,
    IPV6_RECVORIGDSTADDR, IP_RECVORIGDSTADDR, SOL_IP, SOL_IPV6,
};

use crate::{
    rt::net::UdpSocket,
    utils::{new_vec_for_udp, VecExt},
};

fn new_tsock(addr: SocketAddr) -> anyhow::Result<UdpSocket> {
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

    match addr {
        SocketAddr::V4(addr) => nix::sys::socket::bind(socket, &SockaddrIn::from(addr)),
        SocketAddr::V6(addr) => nix::sys::socket::bind(socket, &SockaddrIn6::from(addr)),
    }
    .with_context(|| format!("Binding on {addr}"))?;

    log::debug!("UDP tproxy bound on {addr}");

    Ok(UdpSocket::try_from(unsafe {
        std::net::UdpSocket::from_raw_fd(socket)
    })?)
}

#[allow(dead_code)]
pub fn bind_transparent_udp(
    addr: SocketAddr,
) -> anyhow::Result<
    impl Stream<Item = (Bytes, SocketAddr, SocketAddr)>
        + Sink<(Bytes, SocketAddr, SocketAddr), Error = anyhow::Error>
        + Unpin
        + Send
        + Sync
        + Sized,
> {
    Ok(TransparentUdpSocket {
        main: new_tsock(addr)?,
        senders: Default::default(),
    })
}

fn recv_with_orig_dst(
    socket: RawFd,
    buf: &mut [u8],
) -> std::io::Result<(usize, SocketAddr, SocketAddr)> {
    const V6_ADDR_LEN: usize = size_of::<sockaddr_in6>();
    let mut src_addr: [MaybeUninit<u8>; V6_ADDR_LEN] =
        unsafe { MaybeUninit::uninit().assume_init() };

    let mut dst_addr: [MaybeUninit<u8>; V6_ADDR_LEN] =
        unsafe { MaybeUninit::uninit().assume_init() };

    let mut src_addr_len = src_addr.len() as socklen_t;
    let mut dst_addr_len = src_addr.len() as socklen_t;

    let rc = unsafe {
        do_recv_with_orig_dst(
            socket as c_int,
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
        .ok_or_else(|| std::io::Error::new(ErrorKind::AddrNotAvailable, "SRC addr unavailable"))?;

    let dst = buf_to_addr(dst_addr.as_ptr() as *const c_void, dst_addr_len as usize)
        .ok_or_else(|| std::io::Error::new(ErrorKind::AddrNotAvailable, "DST addr unavailable"))?;

    Ok((rc as usize, src, dst))
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

struct TransparentUdpSocket {
    main: UdpSocket,
    senders: HashMap<SocketAddr, UdpSocket>,
}

impl Stream for TransparentUdpSocket {
    type Item = (Bytes, SocketAddr, SocketAddr);

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        ready!(Pin::new(&self.main).poll_readable(cx));
        let mut buf = new_vec_for_udp();
        match recv_with_orig_dst(self.main.as_raw_fd(), &mut buf) {
            Ok((len, src, dst)) => {
                buf.set_len_uninit(len);
                Poll::Ready(Some((buf, src, dst)))
            }
            Err(e) => Poll::Ready(None),
        }
    }
}

impl Sink<(Bytes, SocketAddr, SocketAddr)> for TransparentUdpSocket {
    type Error = anyhow::Result<()>;

    fn poll_ready(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(
        self: Pin<&mut Self>,
        item: (Bytes, SocketAddr, SocketAddr),
    ) -> Result<(), Self::Error> {
        let (data, dst, orig_dst) = item;
        match self.senders.get(&orig_dst) {
            Some(v) => match Pin::new(v).try_send_to(&data, dst)? {
                Some(_) => Poll::Ready(Ok(())),
                None => Poll::Pending,
            },
            None => {
                let sender = new_tsock(orig_dst)?;
                let r = match Pin::new(&sender).try_send_to(&data, dst)? {
                    Some(_) => Poll::Ready(Ok),
                    None => Poll::Pending,
                };
                self.senders.insert(orig_dst, sender);
                r
            }
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

// impl DatagramSocket for TransparentUdpSocket {
//     type RecvType = (Bytes, SocketAddr, SocketAddr);

//     fn poll_recv(
//         self: std::pin::Pin<&Self>,
//         cx: &mut std::task::Context<'_>,
//     ) -> Poll<std::io::Result<Self::RecvType>> {
//         ready!(Pin::new(&self.0).poll_readable(cx))?;
//         let mut buf = new_vec_for_udp();
//         let (len, src, dst) = recv_with_orig_dst(self.0.as_raw_fd(), &mut buf)?;
//         buf.set_len_uninit(len);
//         Poll::Ready(Ok((buf.into(), src, dst)))
//     }

//     fn poll_send(
//         self: std::pin::Pin<&Self>,
//         cx: &mut std::task::Context<'_>,
//         buf: &[u8],
//         addr: SocketAddr,
//     ) -> Poll<std::io::Result<usize>> {
//         Pin::new(&self.0).poll_send(cx, buf, addr)
//     }

//     fn poll_send_ready(
//         self: Pin<&Self>,
//         cx: &mut std::task::Context<'_>,
//     ) -> Poll<std::io::Result<()>> {
//         Pin::new(&self.0).poll_writable(cx)
//     }
// }
