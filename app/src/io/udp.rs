use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::atomic::{AtomicUsize, Ordering},
};

use derive_more::Deref;

use crate::rt::net::UdpSocket as AsyncUdpSocket;
use crate::socks5::Address;

#[derive(Deref)]
pub struct UdpSocket(AsyncUdpSocket);

static UDP_SOCKET_COUNT: AtomicUsize = AtomicUsize::new(0);

impl UdpSocket {
    pub fn is_v4(&self) -> bool {
        match self.local_addr() {
            Ok(v) => v.is_ipv4(),
            _ => true,
        }
    }

    pub async fn bind(v4: bool) -> std::io::Result<Self> {
        Ok(Self(
            AsyncUdpSocket::bind((
                if v4 {
                    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
                } else {
                    IpAddr::V6(Ipv6Addr::UNSPECIFIED)
                },
                0,
            ))
            .await?,
        ))
    }

    pub async fn send_to_addr(&self, buf: &[u8], addr: &Address<'_>) -> std::io::Result<usize> {
        match addr {
            Address::IP(addr) => self.0.send_to(buf, addr.clone()).await,
            Address::Name { host, port } => self.send_to(buf, (host.as_ref(), *port)).await,
        }
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        let count = UDP_SOCKET_COUNT.fetch_sub(1, Ordering::Acquire);
        log::debug!("Dropping UDP socket. Remaining: {count}");
    }
}
