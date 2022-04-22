use std::{
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Future;

pub trait DatagramSocket {
    type RecvType;

    fn poll_recv(
        self: Pin<&Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<Self::RecvType>>;

    fn poll_send(
        self: Pin<&Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
        addr: SocketAddr,
    ) -> Poll<std::io::Result<usize>>;

    fn recv_dgram<'a>(&'a self, buf: &'a mut [u8]) -> PollRecv<'a, Self>
    where
        Self: Unpin,
    {
        PollRecv { t: self, buf }
    }

    fn send_dgram<'a>(&'a self, buf: &'a [u8], addr: SocketAddr) -> PollSend<'a, Self>
    where
        Self: Unpin,
    {
        PollSend { t: self, buf, addr }
    }
}

pub struct PollRecv<'a, T: ?Sized> {
    t: &'a T,
    buf: &'a mut [u8],
}

pub struct PollSend<'a, T: ?Sized> {
    t: &'a T,
    buf: &'a [u8],
    addr: SocketAddr,
}

impl<'a, T: DatagramSocket + Unpin + ?Sized> Future for PollRecv<'a, T> {
    type Output = std::io::Result<T::RecvType>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(self.t).poll_recv(cx, self.buf)
    }
}

impl<'a, T: DatagramSocket + Unpin + ?Sized> Future for PollSend<'a, T> {
    type Output = std::io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(self.t).poll_send(cx, self.buf, self.addr)
    }
}
