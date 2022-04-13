use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::task::Poll;


use crate::rt::net::UdpSocket;


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
