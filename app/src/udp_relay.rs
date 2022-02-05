use crate::client::ClientStatistics;
use crate::config::ClientConfig;
use crate::io::{copy_udp_and_stream, copy_udp_and_udp, UdpSocket};
use crate::proxy::protocol::{ProxyRequest, ProxyResult};
use crate::proxy::request_proxy_upstream;
use crate::socks5::{Address, UdpPacket};
use futures_lite::{AsyncRead, AsyncWrite};
use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

async fn serve_socks5_udp_stream_relay<'a>(
    socks5_sock: UdpSocket,
    mut socks5_buf: Vec<u8>,
    socks5_remote_addr: SocketAddr,
    mut upstream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'a,
    upstream_tx_count: &'a AtomicUsize,
    upstream_rx_count: &'a AtomicUsize,
) -> anyhow::Result<()> {
    match UdpPacket::parse_udp(socks5_buf.as_slice()) {
        Ok(v) if v.frag_no == 0 => {
            UdpPacket::write_tcp(&mut upstream, &v.addr, v.data.as_ref()).await?;
        }
        _ => {}
    };
    socks5_buf.clear();

    copy_udp_and_stream(
        socks5_sock,
        socks5_buf,
        upstream,
        Some(MAX_STREAM_HDR_LEN),
        |_, hdr_buf, out_buf| match UdpPacket::parse_udp(out_buf.as_slice())? {
            p if p.frag_no == 0 => {
                let mut hdr_buf = hdr_buf.unwrap();
                UdpPacket::write_tcp_headers(&mut hdr_buf, &p.addr, p.data.len())?;
                upstream_tx_count.fetch_add(hdr_buf.len() + out_buf.len(), Ordering::Relaxed);
                Ok(())
            }
            _ => {
                hdr_buf.unwrap().clear();
                out_buf.clear();
                Ok(())
            }
        },
        move |buf, out| match UdpPacket::parse_tcp(buf)? {
            None => Ok(None),
            Some((offset, p)) => {
                out.clear();
                p.write_udp_sync(out)?;
                upstream_rx_count.fetch_add(out.len(), Ordering::Relaxed);
                Ok(Some((offset, Address::IP(socks5_remote_addr.clone()))))
            }
        },
    )
    .await
}

const MAX_SOCKS5_UDP_HDR_LEN: usize = 512;
const MAX_STREAM_HDR_LEN: usize = 512;

async fn serve_socks5_udp_directly(
    socks5_sock: UdpSocket,
    mut socks5_buf: Vec<u8>,
    socks5_remote_addr: SocketAddr,
) -> anyhow::Result<()> {
    let upstream = UdpSocket::bind(socks5_sock.is_v4()).await?;

    match UdpPacket::parse_udp(socks5_buf.as_slice()) {
        Ok(UdpPacket {
            frag_no,
            addr,
            data,
        }) if frag_no == 0 => {
            upstream.send_to_addr(data.as_ref(), &addr).await?;
        }
        _ => {}
    };
    socks5_buf.clear();

    copy_udp_and_udp(
        socks5_sock,
        socks5_buf,
        upstream,
        Some(MAX_SOCKS5_UDP_HDR_LEN),
        None,
        move |_, buf| match UdpPacket::parse_udp(buf.as_slice())? {
            UdpPacket {
                frag_no,
                addr,
                data,
            } if frag_no == 0 => Ok(Some((addr, data))),
            _ => Ok(None),
        },
        move |addr, buf| {
            let addr = Address::IP(addr);
            let hdr_len = UdpPacket::udp_header_len(&addr);
            let hdr_offset = {
                let (hdr, _) = buf.split_at_mut(MAX_SOCKS5_UDP_HDR_LEN);
                let mut hdr = &mut hdr[MAX_SOCKS5_UDP_HDR_LEN - hdr_len..];
                UdpPacket::write_udp_headers(&addr, &mut hdr)?;
                MAX_SOCKS5_UDP_HDR_LEN - hdr_len
            };

            Ok(Some((
                Address::IP(socks5_remote_addr.clone()),
                Cow::Borrowed(&buf.as_slice()[hdr_offset..]),
            )))
        },
    )
    .await
}

pub struct Relay {
    c: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
    socket: UdpSocket,
}

impl Relay {
    pub async fn new(
        c: Arc<ClientConfig>,
        stats: Arc<ClientStatistics>,
        v4: bool,
    ) -> anyhow::Result<(Self, SocketAddr)> {
        let socket = UdpSocket::bind(v4).await?;
        let addr = socket.local_addr()?;
        Ok((Self { c, socket, stats }, addr))
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let Relay { c, socket, stats } = self;

        let mut buf = vec![0; 65536];
        let socks5_remote_addr = match socket.recv_from(buf.as_mut_slice()).await? {
            (v, _) if v == 0 => return Ok(()),
            (v, addr) => {
                buf.resize(v, 0);
                addr
            }
        };

        let UdpPacket { addr, .. } = UdpPacket::parse_udp(buf.as_slice())?;

        // Find out where we want to go
        match c.find_best_upstream(stats.as_ref(), &addr) {
            None => {
                log::debug!("Connecting to udp://{addr} directly");
                serve_socks5_udp_directly(socket, buf, socks5_remote_addr).await
            }
            Some((name, upstream)) => {
                log::debug!("Requesting UDP proxy upstream: {name} for {addr}");
                match request_proxy_upstream(upstream, &ProxyRequest::UDP).await {
                    Ok((ProxyResult::Granted { .. }, upstream)) => {
                        let (tx_count, rx_count) = stats
                            .upstreams
                            .get(name)
                            .map(|stat| (stat.rx.clone(), stat.tx.clone()))
                            .unwrap_or_else(|| (Default::default(), Default::default()));
                        serve_socks5_udp_stream_relay(
                            socket,
                            buf,
                            socks5_remote_addr,
                            upstream,
                            tx_count.as_ref(),
                            rx_count.as_ref(),
                        )
                        .await
                    }
                    Ok((r, _)) => Err(r.into()),
                    Err(e) => Err(e.into()),
                }
            }
        }
    }
}
