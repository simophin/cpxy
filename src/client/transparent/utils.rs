use std::{
    io::ErrorKind,
    mem::{size_of, MaybeUninit},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    os::unix::prelude::{AsRawFd, FromRawFd, RawFd},
    pin::Pin,
    task::Poll,
};

use anyhow::Context;
use bytes::Bytes;
use futures::{ready, Sink, Stream, StreamExt};
use nix::sys::socket::{
    setsockopt, sockopt::IpTransparent, AddressFamily, InetAddr, SockAddr, SockFlag, SockProtocol,
};

use libc::{
    c_int, c_void, size_t, sockaddr_in, sockaddr_in6, socklen_t, ssize_t, AF_INET, AF_INET6,
    IPV6_RECVORIGDSTADDR, IP_RECVORIGDSTADDR, SOL_IP, SOL_IPV6,
};

use crate::{
    io::UdpSocketExt,
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

    nix::sys::socket::bind(socket, &SockAddr::Inet(InetAddr::from_std(&addr)))
        .with_context(|| format!("Binding on {addr}"))?;

    log::debug!("UDP tproxy bound on {addr}");

    Ok(UdpSocket::try_from(unsafe {
        std::net::UdpSocket::from_raw_fd(socket)
    })?)
}

pub fn bind_transparent_udp_for_reciving(
    addr: SocketAddr,
) -> anyhow::Result<impl Stream<Item = (Bytes, SocketAddr, SocketAddr)> + Unpin + Send + Sync + Sized>
{
    Ok(TransparentUdpStream(new_tsock(addr)?))
}

pub fn bind_transparent_udp_for_sending(
    addr: SocketAddr,
) -> anyhow::Result<
    impl Sink<(Bytes, SocketAddr), Error = std::io::Error> + Unpin + Send + Sync + Sized,
> {
    Ok(new_tsock(addr)?.to_sink_stream().split().0)
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

struct TransparentUdpStream(UdpSocket);

impl Stream for TransparentUdpStream {
    type Item = (Bytes, SocketAddr, SocketAddr);

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if ready!(Pin::new(&self.0).poll_readable(cx)).is_err() {
            return Poll::Ready(None);
        }
        let mut buf = new_vec_for_udp();
        match recv_with_orig_dst(self.0.as_raw_fd(), &mut buf) {
            Ok((len, src, dst)) => {
                buf.set_len_uninit(len);
                Poll::Ready(Some((buf.into(), src, dst)))
            }
            Err(_) => Poll::Ready(None),
        }
    }
}
