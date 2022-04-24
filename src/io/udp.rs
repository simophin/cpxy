use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::task::Poll;

use bytes::Bytes;
use futures::ready;

use crate::{
    rt::net::UdpSocket,
    socks5::Address,
    utils::{new_vec_for_udp, VecExt},
};

use super::DatagramSocket;

pub async fn bind_udp(v4: bool) -> std::io::Result<UdpSocket> {
    UdpSocket::bind((
        if v4 {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
        },
        0,
    ))
    .await
}

pub async fn send_to_addr(
    socket: &UdpSocket,
    buf: &[u8],
    addr: &Address<'_>,
) -> std::io::Result<usize> {
    match addr {
        Address::IP(addr) => socket.send_to(buf, addr).await,
        Address::Name { host, port } => socket.send_to(buf, (host.as_ref(), *port)).await,
    }
}

pub trait UdpSocketExt {
    fn is_v4(&self) -> bool;
}

impl UdpSocketExt for UdpSocket {
    fn is_v4(&self) -> bool {
        match self.local_addr() {
            Ok(v) => v.is_ipv4(),
            _ => true,
        }
    }
}

impl DatagramSocket for UdpSocket {
    type RecvType = (Bytes, SocketAddr);

    fn poll_recv(
        self: std::pin::Pin<&Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<Self::RecvType>> {
        ready!(self.poll_readable(cx))?;
        let mut buf = new_vec_for_udp();
        let (len, addr) = match self.try_recv_from(&mut buf)? {
            Some(v) => v,
            None => return Poll::Pending,
        };
        buf.set_len_uninit(len);
        Poll::Ready(Ok((buf.into(), addr)))
    }

    fn poll_send(
        self: std::pin::Pin<&Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
        addr: std::net::SocketAddr,
    ) -> Poll<std::io::Result<usize>> {
        ready!(self.poll_writable(cx))?;
        Poll::Ready(match self.try_send_to(buf, addr) {
            Ok(Some(v)) => Ok(v),
            Ok(None) => return Poll::Pending,
            Err(e) => Err(e),
        })
    }
}
