use bytes::{Bytes, BytesMut};
use either::Either;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use crate::addr::Address;

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

pub async fn connect_tcp(a: &Address) -> std::io::Result<TcpStream> {
    match a.domain_or_ip() {
        Either::Left(addr) => TcpStream::connect(addr).await,
        Either::Right(addr) => TcpStream::connect(addr).await,
    }
}

pub async fn connect_tcp_marked(a: &Address, fwmark: Option<u32>) -> std::io::Result<TcpStream> {
    let stream = connect_tcp(a).await?;
    if let Some(mark) = fwmark {
        stream.set_sock_mark(mark)?;
    }
    Ok(stream)
}

pub async fn bind_tcp(a: &Address) -> std::io::Result<TcpListener> {
    match a.domain_or_ip() {
        Either::Right(addr) => TcpListener::bind(addr).await,
        Either::Left(addr) => TcpListener::bind(addr).await,
    }
}

pub async fn read_data_with_timeout<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> anyhow::Result<Option<Bytes>> {
    let mut buf = BytesMut::zeroed(4096);
    match timeout(Duration::from_millis(200), stream.read(&mut buf)).await {
        Ok(Ok(n)) => {
            buf.truncate(n);
            Ok(Some(buf.freeze()))
        }
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Ok(None),
    }
}
