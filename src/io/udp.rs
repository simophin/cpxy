use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use bytes::Bytes;
use futures::{ready, Sink, Stream};

use crate::utils::{new_vec_for_udp, VecExt};
use crate::{rt::net::UdpSocket, socks5::Address};

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

pub fn is_one_off_udp_query(dst: &Address<'_>) -> bool {
    dst.get_port() == 53
}

pub trait UdpSocketExt {
    fn is_v4(&self) -> bool;
    fn to_sink_stream(self) -> UdpSocketSinkStream;
}

impl UdpSocketExt for UdpSocket {
    fn is_v4(&self) -> bool {
        match self.local_addr() {
            Ok(v) => v.is_ipv4(),
            _ => true,
        }
    }

    fn to_sink_stream(self) -> UdpSocketSinkStream {
        UdpSocketSinkStream {
            socket: self,
            buffer_waker: None,
            buffers: Default::default(),
        }
    }
}

pub struct UdpSocketSinkStream {
    socket: UdpSocket,
    buffers: VecDeque<(Bytes, SocketAddr)>,
    buffer_waker: Option<Waker>,
}

impl Stream for UdpSocketSinkStream {
    type Item = (Bytes, SocketAddr);

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match ready!(Pin::new(&self.socket).poll_readable(cx)) {
            Ok(_) => {}
            Err(_) => return Poll::Ready(None),
        };

        let mut buf = new_vec_for_udp();
        match self.socket.try_recv_from(&mut buf) {
            Ok(None) => Poll::Pending,
            Ok(Some((len, addr))) => {
                buf.set_len_uninit(len);
                Poll::Ready(Some((buf.into(), addr)))
            }
            Err(_) => Poll::Ready(None),
        }
    }
}

const MAX_BUFFER_SIZE: usize = 10;

impl Sink<(Bytes, SocketAddr)> for UdpSocketSinkStream {
    type Error = std::io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.buffers.len() >= MAX_BUFFER_SIZE {
            self.buffer_waker.replace(cx.waker().clone());
            return Poll::Pending;
        }
        return Poll::Ready(Ok(()));
    }

    fn start_send(mut self: Pin<&mut Self>, item: (Bytes, SocketAddr)) -> Result<(), Self::Error> {
        self.buffers.push_back(item);
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(Pin::new(&self.socket).poll_writable(cx))?;
        while let Some((data, addr)) = self.buffers.front() {
            match self.socket.try_send_to(data.as_ref(), *addr) {
                Ok(_) => {
                    let _ = self.buffers.pop_front();
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Poll::Pending,
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
        if let Some(waker) = self.buffer_waker.take() {
            waker.wake();
        }
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}
