use crate::io::UdpSocket;
use crate::socks5::UdpPacket;
use crate::utils::RWBuffer;
use futures_lite::future::race;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite};
use smol::spawn;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

pub async fn copy_socks5_udp_to_stream(
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

pub async fn copy_stream_to_socks5_udp(
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

pub async fn serve_socks5_udp_stream_relay(
    socks5_sock: UdpSocket,
    upstream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let (r, w) = split(upstream);
    let last_addr: Arc<RwLock<Option<SocketAddr>>> = Default::default();
    let socket = Arc::new(socks5_sock);
    let task1 = {
        let socket = socket.clone();
        let last_addr = last_addr.clone();
        spawn(async move { copy_socks5_udp_to_stream(&socket, w, last_addr).await })
    };

    let task2 = {
        let socket = socket.clone();
        let last_addr = last_addr.clone();
        spawn(async move { copy_stream_to_socks5_udp(r, &socket, last_addr).await })
    };

    race(task1, task2).await
}
