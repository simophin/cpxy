use std::net::SocketAddr;

use crate::socks5::Address;

use crate::rt::net::{TcpListener, TcpStream};

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
        use std::{net::SocketAddrV4, os::unix::prelude::AsRawFd};

        use nix::sys::socket::{getsockopt, sockopt::OriginalDst};
        let addr = getsockopt(self.as_raw_fd(), OriginalDst).ok()?;

        let addr = SocketAddr::V4(SocketAddrV4::new(
            u32::from_be(addr.sin_addr.s_addr).into(),
            u16::from_be(addr.sin_port),
        ));

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
