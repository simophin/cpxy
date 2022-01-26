use crate::io::UdpSocket;
use crate::proxy::protocol::ProxyResult;
use crate::socks5::{Address, UdpPacket};
use crate::utils::{write_json_lengthed_async, RWBuffer};
use anyhow::anyhow;
use futures_lite::future::race;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite};
use smol::spawn;
use smol_timeout::TimeoutExt;
use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;

async fn copy_stream_to_udp(
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

async fn copy_udp_to_stream(
    socket: &UdpSocket,
    mut dst: impl AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 65536];

    loop {
        let (n, addr) = match socket
            .recv_from(buf.as_mut_slice())
            .timeout(Duration::from_secs(120))
            .await
        {
            Some(Ok(v)) => v,
            None => return Err(anyhow!("UDP socket's idle timeout")),
            Some(Err(e)) => return Err(e.into()),
        };

        let buf = &buf.as_slice()[..n];
        UdpPacket::write_tcp(&mut dst, &Address::IP(addr), buf).await?;
    }
}

pub async fn serve_udp_proxy(
    mut src: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    is_v4: bool,
) -> anyhow::Result<()> {
    log::info!("Proxying UDP upstream");
    let socket = match UdpSocket::bind(is_v4).await {
        Ok(v) => {
            write_json_lengthed_async(
                &mut src,
                ProxyResult::Granted {
                    bound_address: v
                        .local_addr()
                        .ok()
                        .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap()),
                },
            )
            .await?;
            Arc::new(UdpSocket::from(v))
        }
        Err(e) => {
            write_json_lengthed_async(&mut src, ProxyResult::ErrGeneric { msg: e.to_string() })
                .await?;
            return Err(e.into());
        }
    };

    let (r, w) = split(src);

    let task1 = {
        let socket = socket.clone();
        spawn(async move {
            if let Err(e) = copy_stream_to_udp(r, &socket).await {
                log::error!("Error serving UDP upstream: {e}")
            }
            log::info!("Finished serving UDP upstream");
            Ok(())
        })
    };

    let task2 = {
        let socket = socket.clone();
        spawn(async move {
            if let Err(e) = copy_udp_to_stream(&socket, w).await {
                log::error!("Error serving UDP downstream: {e}")
            }
            log::info!("Finished serving UDP upstream");
            Ok(())
        })
    };

    race(task1, task2).await
}
