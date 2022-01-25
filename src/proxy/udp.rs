use crate::proxy::handler::ProxyResult;
use crate::socks5::{Address, UdpPacket};
use crate::utils::RWBuffer;
use futures_lite::future::race;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite};
use smol::net::UdpSocket;
use smol::spawn;
use std::fmt::Write;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

pub async fn copy_from_socks5_udp(
    socket: &UdpSocket,
    mut dst: impl AsyncWrite + Unpin + Send + Sync + 'static,
    last_addr: Arc<RwLock<Option<SocketAddr>>>,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 65536];
    loop {
        let (n, addr) = socket.recv_from(buf.as_mut_slice()).await?;
        log::debug!("SOCKS5-UDP: Received {n} bytes from {addr}");
        let should_write = match last_addr.try_read() {
            Ok(g) if g.as_ref() != Some(&addr) => true,
            _ => false,
        };

        if should_write {
            if let Ok(mut g) = last_addr.try_write() {
                *g = Some(addr);
            }
        }

        let UdpPacket {
            frag_no,
            addr,
            data,
        } = UdpPacket::parse_udp(&buf.as_slice()[..n])?;
        if frag_no != 0 {
            log::info!("Ignoring fragmented packet");
            continue;
        }

        log::debug!(
            "SOCKS5-UDP: Received data(bytes={}) sending to {addr}",
            data.as_ref().len()
        );
        UdpPacket::write_tcp(&mut dst, &addr, data.as_ref()).await?;
    }
}

pub async fn copy_to_socks5_udp(
    mut src: impl AsyncRead + Unpin + Send + Sync + 'static,
    socket: &UdpSocket,
    last_addr: Arc<RwLock<Option<SocketAddr>>>,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::with_capacity(66000);
    let mut udp_buf = Vec::<u8>::new();
    loop {
        match src.read(buf.write_buf()).await? {
            0 => return Ok(()),
            v => buf.advance_write(v),
        };

        while let Some((offset, pkt)) = UdpPacket::parse_tcp(buf.read_buf())? {
            udp_buf.clear();
            pkt.write_udp(&mut udp_buf).await?;
            log::debug!(
                "SOCKS5-UDP: Returning {} bytes to {}",
                pkt.data.as_ref().len(),
                pkt.addr
            );
            drop(pkt);
            buf.advance_read(offset);

            let addr = match last_addr.try_read() {
                Ok(g) if g.is_some() => g.as_ref().unwrap().clone(),
                _ => {
                    log::warn!("No address to send packet to");
                    break;
                }
            };

            socket.send_to(udp_buf.as_slice(), addr).await?;
        }

        if buf.should_compact() {
            buf.compact();
        }
    }
}

async fn copy_packet_to_udp(
    mut src: impl AsyncRead + Unpin + Send + Sync + 'static,
    socket: &UdpSocket,
) -> anyhow::Result<()> {
    let mut buf = RWBuffer::with_capacity(66000);
    let mut addr_buf = String::new();
    loop {
        match src.read(buf.write_buf()).await? {
            0 => return Ok(()),
            v => buf.advance_write(v),
        };

        while let Some((offset, pkt)) = UdpPacket::parse_tcp(buf.read_buf())? {
            log::debug!("UDP-Upstream: sending packet to {}", pkt.addr);
            match &pkt.addr {
                Address::IP(addr) => socket.send_to(pkt.data.as_ref(), addr).await?,
                Address::Name { host, port } => {
                    addr_buf.clear();
                    addr_buf.write_fmt(format_args!("{host}:{port}"))?;
                    socket.send_to(pkt.data.as_ref(), addr_buf.as_str()).await?
                }
            };

            drop(pkt);
            buf.advance_read(offset);
        }
    }
}

async fn copy_packet_to_stream(
    socket: &UdpSocket,
    mut dst: impl AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 65536];

    loop {
        let (n, addr) = socket.recv_from(buf.as_mut_slice()).await?;
        let buf = &buf.as_slice()[..n];
        UdpPacket::write_tcp(&mut dst, &Address::IP(addr), buf).await?;
    }
}

pub async fn serve_udp_proxy(
    target: Address,
    mut src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    log::info!("Proxying UDP upstream: {target}");
    let socket = match UdpSocket::bind("localhost:0").await {
        Ok(v) => {
            super::handler::send_proxy_result(
                &mut src,
                ProxyResult::Granted {
                    bound_address: v
                        .local_addr()
                        .ok()
                        .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap()),
                },
            )
            .await?;
            Arc::new(v)
        }
        Err(e) => {
            super::handler::send_proxy_result(
                &mut src,
                ProxyResult::ErrGeneric { msg: e.to_string() },
            )
            .await?;
            return Err(e.into());
        }
    };

    let (r, w) = split(src);

    let task1 = {
        let socket = socket.clone();
        spawn(async move {
            if let Err(e) = copy_packet_to_udp(r, &socket).await {
                log::error!("Error serving UDP upstream: {e}")
            }
            log::info!("Finished serving UDP upstream");
            Ok(())
        })
    };

    let task2 = {
        let socket = socket.clone();
        spawn(async move {
            if let Err(e) = copy_packet_to_stream(&socket, w).await {
                log::error!("Error serving UDP downstrem: {e}")
            }
            log::info!("Finished serving UDP upstream");
            Ok(())
        })
    };

    race(task1, task2).await
}
