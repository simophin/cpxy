use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::Context;

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
