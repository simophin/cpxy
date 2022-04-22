use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use async_trait::async_trait;
use bytes::Bytes;

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

#[async_trait]
impl DatagramSocket for UdpSocket {
    type RecvType = (Bytes, SocketAddr);

    async fn send_dgram(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<usize> {
        self.send_to(buf, addr).await
    }

    async fn recv_dgram(&self) -> std::io::Result<Self::RecvType> {
        let mut buf = new_vec_for_udp();
        let (len, from) = self.recv_from(&mut buf).await?;
        buf.set_len_uninit(len);
        Ok((buf.into(), from))
    }
}
