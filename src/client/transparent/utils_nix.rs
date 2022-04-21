use std::{
    io::ErrorKind,
    mem::{size_of, MaybeUninit},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    os::unix::prelude::{AsRawFd, FromRawFd},
    task::Poll,
};

use anyhow::Context;
use nix::sys::socket::{
    setsockopt, sockopt::IpTransparent, AddressFamily, InetAddr, MsgFlags, SockAddr, SockFlag,
    SockProtocol,
};

use libc::{
    c_int, c_void, size_t, sockaddr_in, sockaddr_in6, socklen_t, ssize_t, AF_INET, AF_INET6,
    IPV6_RECVORIGDSTADDR, IP_RECVORIGDSTADDR, SOL_IP, SOL_IPV6,
};

use crate::rt::net::UdpSocket;

use super::TransparentUdpSocket;

#[allow(dead_code)]
pub fn bind_transparent_udp(
    addr: SocketAddr,
) -> anyhow::Result<impl TransparentUdpSocket + Unpin + Send + Sync + Sized> {
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

fn recv_with_orig_dst(
    socket: &UdpSocket,
    buf: &mut [u8],
) -> std::io::Result<((usize, SocketAddr), SocketAddr)> {
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
        .ok_or_else(|| std::io::Error::new(ErrorKind::AddrNotAvailable, "SRC addr unavailable"))?;

    let dst = buf_to_addr(dst_addr.as_ptr() as *const c_void, dst_addr_len as usize)
        .ok_or_else(|| std::io::Error::new(ErrorKind::AddrNotAvailable, "DST addr unavailable"))?;

    Ok(((rc as usize, src), dst))
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

impl TransparentUdpSocket for UdpSocket {
    fn poll_recv(
        self: std::pin::Pin<&Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<((usize, SocketAddr), SocketAddr)>> {
        match self.poll_readable(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        };

        Poll::Ready(recv_with_orig_dst(&self, buf))
    }

    fn poll_send_to(
        self: std::pin::Pin<&Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
        addr: SocketAddr,
    ) -> Poll<std::io::Result<usize>> {
        match self.poll_writable(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        };

        Poll::Ready(
            nix::sys::socket::sendto(
                self.as_raw_fd(),
                buf,
                &SockAddr::Inet(InetAddr::from_std(&addr)),
                MsgFlags::MSG_DONTWAIT,
            )
            .map_err(|errno| std::io::Error::from_raw_os_error(errno as i32)),
        )
    }
}
