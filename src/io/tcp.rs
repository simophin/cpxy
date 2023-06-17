use std::net::SocketAddr;

use smol::net::{TcpListener, TcpStream};

use crate::socks5::Address;

use super::AsRawFdExt;

pub trait TcpStreamExt {
    fn is_v4(&self) -> bool;
    fn get_original_dst(&self) -> Option<SocketAddr>;
}

impl TcpStreamExt for TcpStream {
    fn is_v4(&self) -> bool {
        match self.local_addr() {
            Ok(v) => v.is_ipv4(),
            _ => true,
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn get_original_dst(&self) -> Option<SocketAddr> {
        None
    }

    #[cfg(target_os = "linux")]
    fn get_original_dst(&self) -> Option<SocketAddr> {
        use nix::sys::socket::{getsockopt, sockopt::Ip6tOriginalDst, sockopt::OriginalDst};
        use std::{net::SocketAddrV4, net::SocketAddrV6, os::unix::prelude::AsRawFd};

        let addr = if self.is_v4() {
            let addr = getsockopt(self.as_raw_fd(), OriginalDst).ok()?;

            SocketAddr::V4(SocketAddrV4::new(
                u32::from_be(addr.sin_addr.s_addr).into(),
                u16::from_be(addr.sin_port),
            ))
        } else {
            let addr = getsockopt(self.as_raw_fd(), Ip6tOriginalDst).ok()?;

            SocketAddr::V6(SocketAddrV6::new(
                u128::from_be_bytes(addr.sin6_addr.s6_addr).into(),
                u16::from_be(addr.sin6_port),
                0,
                0,
            ))
        };

        match self.local_addr() {
            Ok(a) if a == addr => None,
            _ => Some(addr),
        }
    }
}

pub async fn connect_tcp(a: &Address<'_>) -> std::io::Result<TcpStream> {
    match a {
        Address::IP(addr) => Ok(TcpStream::connect(addr).await?),
        Address::Name { host, port } => Ok(TcpStream::connect((host.as_ref(), *port)).await?),
    }
}

pub async fn connect_tcp_marked(
    a: &Address<'_>,
    fwmark: Option<u32>,
) -> std::io::Result<TcpStream> {
    let stream = connect_tcp(a).await?;
    if let Some(mark) = fwmark {
        stream.set_sock_mark(mark)?;
    }
    Ok(stream)
}

pub async fn bind_tcp(a: &Address<'_>) -> std::io::Result<TcpListener> {
    match a {
        Address::IP(addr) => Ok(TcpListener::bind(addr).await?),
        Address::Name { host, port } => Ok(TcpListener::bind((host.as_ref(), *port)).await?),
    }
}
