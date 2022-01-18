use crate::utils::RWBuffer;
use anyhow::anyhow;
use bytes::{Buf, BufMut, Bytes};
use pin_project_lite::pin_project;
use std::borrow::{Borrow, Cow};
use std::cmp::min;
use std::collections::VecDeque;
use std::io::{Cursor, Error, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, Sink};
use tokio::net::UdpSocket;
use url::Url;

#[derive(Clone)]
pub struct Frame(pub SocketAddr, pub Bytes);

impl Frame {
    fn parse_stream(buf: &[u8]) -> anyhow::Result<Option<(usize, SocketAddr, &[u8])>> {
        let mut buf = Cursor::new(buf);
        let addr_type = if buf.remaining() < 1 {
            return Ok(None);
        } else {
            buf.get_u8()
        };

        let addr = match addr_type {
            0 if buf.remaining() >= 6 => {
                let mut b = [0u8; 4];
                buf.read_exact(&mut b)?;
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(b), buf.get_u16()))
            }
            1 if buf.remaining() >= 18 => {
                let mut b = [0u8; 16];
                buf.read_exact(&mut b)?;
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from(b), buf.get_u16(), 0, 0))
            }
            0 | 1 => return Ok(None),
            v => return Err(anyhow!("unexpected address type: {v}, expecting 0 or 1")),
        };

        let len = if buf.remaining() < 2 {
            return Ok(None);
        } else {
            buf.get_u16() as usize
        };

        if buf.remaining() < len {
            return Ok(None);
        }

        let buf_position = buf.position() as usize;
        let buf = &buf.into_inner()[buf_position..buf_position + len];
        Ok(Some((buf_position + len, addr, buf)))
    }
}

pin_project! {
    struct UdpFrameStream {
        #[pin]
        inner: UdpSocket,
        rdbuf: RWBuffer,
        wrbuf: RWBuffer,
        pending_writes: VecDeque<Frame>,
    }
}

const V6_HDR_SIZE: usize = 1 + 16 + 2;
const V4_HDR_SIZE: usize = 1 + 4 + 2;

impl AsyncRead for UdpFrameStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.rdbuf.remaining_read() > 0 {
            let len = min(buf.remaining(), self.rdbuf.remaining_read());
            buf.put_slice(&self.rdbuf.read_buf()[..len]);
            self.rdbuf.advance_read(len);
            cx.waker().wake_by_ref();
            return Poll::Ready(Ok(()));
        }

        let mut this = self.project();
        let mut udp_buf = ReadBuf::new(&mut this.rdbuf.write_buf()[V6_HDR_SIZE..]);
        match this.inner.as_mut().poll_recv_from(cx, &mut udp_buf) {
            Poll::Ready(Ok(SocketAddr::V4(addr))) => {
                let n = udp_buf.filled().len();
                if n > 0 {
                    drop(udp_buf);
                    this.rdbuf.advance_write(n + V6_HDR_SIZE);
                    this.rdbuf.advance_read(V6_HDR_SIZE - V4_HDR_SIZE);
                    let mut hdr_buf = this.rdbuf.read_buf_mut();
                    hdr_buf.put_u8(0);
                    hdr_buf.put_slice(&addr.ip().octets());
                    hdr_buf.put_u16(addr.port());
                }
            }
            Poll::Ready(Ok(SocketAddr::V6(addr))) => {
                let n = udp_buf.filled().len();
                if n > 0 {
                    drop(udp_buf);
                    this.rdbuf.advance_write(n + V6_HDR_SIZE);
                    let mut hdr_buf = this.rdbuf.read_buf_mut();
                    hdr_buf.put_u8(1);
                    hdr_buf.put_slice(&addr.ip().octets());
                    hdr_buf.put_u16(addr.port());
                }
            }
            Poll::Ready(Err(e)) => return Poll::Ready((Err(e))),
            Poll::Pending => return Poll::Pending,
        };

        if self.rdbuf.remaining_read() > 0 {
            let len = min(buf.remaining(), self.rdbuf.remaining_read());
            udp_buf.put_slice(&self.rdbuf.read_buf()[..len]);
            self.rdbuf.advance_read(len);
            if self.rdbuf.remaining_read() > 0 {
                cx.waker().wake_by_ref();
            }
        }
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for UdpFrameStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        let mut this = self.project();

        // Send pending frames
        while let Some(Frame(addr, data)) = this.pending_writes.front() {
            match this
                .inner
                .as_mut()
                .poll_send_to(cx, data.as_ref(), addr.clone())
            {
                Poll::Ready(_) => this.pending_writes.pop_front(),
                Poll::Pending => break,
            };
        }

        let n = min(this.wrbuf.remaining_write(), buf.len());
        this.wrbuf.write(&buf[..n]);

        while this.wrbuf.remaining_read() > 0 {
            match Frame::parse_stream(this.wrbuf.read_buf()) {
                Ok(None) => {
                    // Partial frame, stop here
                    break;
                }
                Ok(Some((offset, addr, data))) => {
                    match this.inner.as_mut().poll_send_to(cx, data, addr.clone()) {
                        Poll::Pending => {
                            // The underlying socket can't accept this frame, push it to pending list
                            this.pending_writes
                                .push_back(Frame(addr, Bytes::copy_from_slice(data)));
                            this.wrbuf.advance_read(offset);
                            cx.waker().wake_by_ref();
                            break;
                        }
                        Poll::Ready(Ok(_)) => this.wrbuf.advance_read(offset),
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    }
                }
                Err(e) => return Poll::Ready(Err(e.into())),
            }
        }

        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let mut this = self.project();

        // Send pending writes
        while let Some(Frame(addr, data)) = this.pending_writes.front() {
            match this
                .inner
                .as_mut()
                .poll_send_to(cx, data.as_ref(), addr.clone())
            {
                Poll::Ready(_) => this.pending_writes.pop_front(),
                Poll::Pending => {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
            };
        }

        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        todo!()
    }
}

pub fn is_udp(url: &Url) -> bool {
    return url.scheme().eq_ignore_ascii_case("udp") && url.has_host() && url.port().is_some();
}

pub async fn udp_fetcher(
    url: Url,
) -> anyhow::Result<(SocketAddr, impl AsyncRead + AsyncWrite + Unpin)> {
    assert!(is_udp(&url));
    let addr = format!("{}:{}", url.host_str().unwrap(), url.port().unwrap());
    log::info!("Connecting to UDP://{addr}");

    let mut socket = UdpSocket::bind("127.0.0.1:0").await?;
    socket.connect(addr).await?;
    todo!()
}
