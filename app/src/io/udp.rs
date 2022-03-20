use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::Context;
use libc::EWOULDBLOCK;
use nix::errno::errno;
use smol::Async;

use crate::socks5::Address;

type StdUdpSocket = std::net::UdpSocket;

pub struct UdpSocket(Async<StdUdpSocket>);

static UDP_SOCKET_COUNT: AtomicUsize = AtomicUsize::new(0);

impl TryFrom<StdUdpSocket> for UdpSocket {
    type Error = anyhow::Error;

    fn try_from(s: StdUdpSocket) -> anyhow::Result<Self> {
        UDP_SOCKET_COUNT.fetch_add(1, Ordering::Acquire);
        Ok(Self(Async::new(s)?))
    }
}

impl UdpSocket {
    pub fn is_v4(&self) -> bool {
        match self.0.as_ref().local_addr() {
            Ok(v) => v.is_ipv4(),
            _ => true,
        }
    }

    pub fn bind_sync(addr: &SocketAddr) -> anyhow::Result<Self> {
        StdUdpSocket::bind(addr)?.try_into()
    }

    pub async fn bind(v4: bool) -> anyhow::Result<Self> {
        Self::bind_addr(if v4 {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
        })
        .await
    }

    pub async fn bind_raw(addr: &SocketAddr) -> anyhow::Result<Self> {
        StdUdpSocket::bind(addr)?.try_into()
    }

    pub async fn bind_addr(ip: IpAddr) -> anyhow::Result<Self> {
        Self::bind_raw(&SocketAddr::new(ip, 0)).await
    }

    pub async fn send_to_addr(&self, buf: &[u8], addr: &Address<'_>) -> anyhow::Result<usize> {
        match addr {
            Address::IP(addr) => Ok(self.0.send_to(buf, addr.clone()).await?),
            Address::Name { host, port } => self.send_to(buf, (host.as_ref(), *port)).await,
        }
    }

    pub async fn send_to(
        &self,
        buf: &[u8],
        addr: impl smol::net::AsyncToSocketAddrs,
    ) -> anyhow::Result<usize> {
        let addr = addr
            .to_socket_addrs()
            .await?
            .next()
            .context("Resolving address")?;
        Ok(self.0.send_to(buf, addr).await?)
    }

    pub async fn wait_read(&self) -> anyhow::Result<()> {
        futures_lite::future::poll_fn(|ctx| self.0.poll_readable(ctx)).await?;
        Ok(())
    }

    pub async fn recv_from(&self, buf: &mut [u8]) -> anyhow::Result<(usize, SocketAddr)> {
        Ok(self.0.recv_from(buf).await?)
    }

    #[cfg(unix)]
    pub fn set_trasnparent(&self) -> anyhow::Result<()> {
        use std::{ffi::CStr, mem::size_of_val, os::unix::prelude::AsRawFd};

        use anyhow::bail;
        use libc::{c_void, setsockopt, socklen_t, strerror, IPPROTO_IP, IP_TRANSPARENT};
        use nix::errno::errno;

        let value = 1usize;
        let rc = unsafe {
            setsockopt(
                self.0.as_raw_fd(),
                IPPROTO_IP,
                IP_TRANSPARENT,
                &value as *const usize as *const c_void,
                size_of_val(&value) as socklen_t,
            )
        };

        if rc != 0 {
            bail!("Unable to set transparent proxy: {}", unsafe {
                CStr::from_ptr(strerror(errno()) as *const i8)
                    .to_str()
                    .unwrap()
            });
        }
        Ok(())
    }

    #[cfg(unix)]
    pub fn set_receive_original_dst(&self) -> anyhow::Result<()> {
        use std::{mem::size_of, os::unix::prelude::AsRawFd};

        use anyhow::bail;
        use libc::{
            c_void, setsockopt, socklen_t, IPV6_RECVORIGDSTADDR, IP_RECVORIGDSTADDR, SOL_IP,
            SOL_IPV6,
        };

        let value = 1usize;
        let rc = if self.is_v4() {
            unsafe {
                setsockopt(
                    self.0.as_raw_fd(),
                    SOL_IP,
                    IP_RECVORIGDSTADDR,
                    &value as *const usize as *const c_void,
                    size_of::<usize>() as socklen_t,
                )
            }
        } else {
            unsafe {
                setsockopt(
                    self.0.as_raw_fd(),
                    SOL_IPV6,
                    IPV6_RECVORIGDSTADDR,
                    &value as *const usize as *const c_void,
                    size_of::<usize>() as socklen_t,
                )
            }
        };
        if rc != 0 {
            bail!("Unable to set IP_RECVORIGDSTADDR: {rc}");
        }
        Ok(())
    }

    #[cfg(unix)]
    fn buf_to_v6(buf: *const libc::c_void, buf_len: usize) -> Option<SocketAddr> {
        use libc::sockaddr_in6;
        use std::net::SocketAddrV6;

        if buf_len < std::mem::size_of::<sockaddr_in6>() {
            return None;
        }

        let addr = unsafe { &*(buf as *const sockaddr_in6) };
        Some(SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::from(addr.sin6_addr.s6_addr),
            u16::from_be(addr.sin6_port),
            u32::from_be(addr.sin6_flowinfo),
            u32::from_be(addr.sin6_scope_id),
        )))
    }

    #[cfg(unix)]
    fn buf_to_v4(buf: *const libc::c_void, buf_len: usize) -> Option<SocketAddr> {
        use libc::sockaddr_in;
        use std::net::SocketAddrV4;

        if buf_len < std::mem::size_of::<sockaddr_in>() {
            return None;
        }

        let addr = unsafe { &*(buf as *const sockaddr_in) };
        Some(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr)),
            u16::from_be(addr.sin_port),
        )))
    }

    #[cfg(unix)]
    pub fn get_original_dst(&self) -> Option<SocketAddr> {
        use std::{net::SocketAddrV4, os::unix::prelude::AsRawFd};

        use nix::sys::socket::{getsockopt, sockopt::OriginalDst};
        let addr = getsockopt(self.0.as_raw_fd(), OriginalDst).ok()?;

        let addr = SocketAddr::V4(SocketAddrV4::new(
            u32::from_be(addr.sin_addr.s_addr).into(),
            u16::from_be(addr.sin_port),
        ));

        Some(addr)
    }

    #[cfg(unix)]
    pub async fn recvmsg(
        &self,
        buf: &mut [u8],
    ) -> anyhow::Result<(usize, SocketAddr, Option<SocketAddr>)> {
        futures_lite::future::poll_fn(|ctx| self.0.poll_readable(ctx)).await?;
        self.recvmsg_sync(buf)?.context("No readable message")
    }

    pub fn recvmsg_sync(
        &self,
        buf: &mut [u8],
    ) -> anyhow::Result<Option<(usize, SocketAddr, Option<SocketAddr>)>> {
        use std::{cell::RefCell, ffi::CStr, mem::size_of, os::unix::prelude::AsRawFd};

        use anyhow::bail;
        use libc::{
            c_void, cmsghdr, iovec, msghdr, recvmsg, sockaddr_in, sockaddr_in6, strerror,
            CMSG_DATA, CMSG_FIRSTHDR, CMSG_NXTHDR, CMSG_SPACE, IPV6_ORIGDSTADDR, IP_ORIGDSTADDR,
            MSG_DONTWAIT, SOL_IP, SOL_IPV6,
        };

        thread_local! {
            static CMSG_BUF: RefCell<Vec<u8>>  = RefCell::new(vec![0u8; unsafe { CMSG_SPACE(size_of::<sockaddr_in6>() as u32) } as usize]);
        }

        CMSG_BUF.with(|v| {
            let mut cmsg_buf = v.borrow_mut();
            let mut received_addr = [0u8; std::mem::size_of::<sockaddr_in6>()];

            let mut iov = iovec {
                iov_base: buf.as_mut_ptr() as *mut c_void,
                iov_len: buf.len(),
            };

            let mut hdr = msghdr {
                msg_name: received_addr.as_mut_ptr() as *mut c_void,
                msg_namelen: received_addr.len() as u32,
                msg_iov: &mut iov,
                msg_iovlen: 1,
                msg_control: cmsg_buf.as_mut_ptr() as *mut c_void,
                msg_controllen: cmsg_buf.len(),
                msg_flags: 0,
            };
            let rc = unsafe { recvmsg(self.0.as_raw_fd(), &mut hdr, MSG_DONTWAIT) };

            if rc < 0 {
                let err = errno();
                if err == EWOULDBLOCK {
                    return Ok(None);
                }
                bail!("Error calling recvmsg, error = {:?}", unsafe {
                    CStr::from_ptr(strerror(err)).to_str()
                });
            }

            let received_addr = match hdr.msg_namelen as usize {
                len if len == size_of::<sockaddr_in6>() => {
                    Self::buf_to_v6(hdr.msg_name, hdr.msg_namelen as usize).unwrap()
                }
                len if len == size_of::<sockaddr_in>() => {
                    Self::buf_to_v4(hdr.msg_name, hdr.msg_namelen as usize).unwrap()
                }
                len => bail!("Invalid received message len {len}"),
            };

            let received_len = rc as usize;

            unsafe {
                let mut cmsg = CMSG_FIRSTHDR(&hdr);
                while cmsg != std::ptr::null_mut() {
                    let cmsghdr {
                        cmsg_level,
                        cmsg_type,
                        cmsg_len,
                    } = &*cmsg;
                    if *cmsg_level == SOL_IP && *cmsg_type == IP_ORIGDSTADDR {
                        return Ok(Some((
                            received_len,
                            received_addr,
                            Self::buf_to_v4(CMSG_DATA(cmsg) as *const c_void, *cmsg_len),
                        )));
                    } else if *cmsg_level == SOL_IPV6 && *cmsg_type == IPV6_ORIGDSTADDR {
                        return Ok(Some((
                            received_len,
                            received_addr,
                            Self::buf_to_v6(CMSG_DATA(cmsg) as *const c_void, *cmsg_len),
                        )));
                    }
                    cmsg = CMSG_NXTHDR(&hdr, cmsg);
                }
            }

            Ok(Some((received_len, received_addr, self.get_original_dst())))
        })
    }

    pub fn local_addr(&self) -> anyhow::Result<SocketAddr> {
        Ok(self.0.as_ref().local_addr()?)
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        let count = UDP_SOCKET_COUNT.fetch_sub(1, Ordering::Acquire);
        log::debug!("Dropping UDP socket. Remaining: {count}");
    }
}
