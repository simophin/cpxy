use std::{net::IpAddr, pin::Pin};

use anyhow::Context;
use futures_lite::{AsyncRead, AsyncWrite};
use smol::Async;

pub struct Device(Async<tun::platform::linux::Device>, usize);

impl Device {
    pub fn new(ip: IpAddr, netmask: IpAddr, mtu: usize) -> anyhow::Result<Self> {
        use tun::{create, platform::linux::Configuration as PlatformConfig, Configuration, Layer};
        let mut config = Configuration::default();
        config
            .layer(Layer::L3)
            .platform(|ctx: &mut PlatformConfig| {
                ctx.packet_information(false);
            })
            .address(ip)
            .netmask(netmask)
            .mtu(mtu as i32)
            .up();
        let value = create(&config).context("Creating tun device")?;
        value.set_nonblock()?;
        Ok(Self(Async::new(value)?, mtu))
    }

    pub fn mtu(&self) -> usize {
        self.1
    }
}

impl AsyncRead for Device {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }

    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_read_vectored(cx, bufs)
    }
}

impl AsyncWrite for Device {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_close(cx)
    }
}
