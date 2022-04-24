use std::{
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::{ready, Future, Sink, Stream};

pub trait DatagramSocket {
    type RecvType;

    fn poll_recv(self: Pin<&Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<Self::RecvType>>;

    fn poll_send(
        self: Pin<&Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
        addr: SocketAddr,
    ) -> Poll<std::io::Result<usize>>;

    fn recv_dgram<'a>(&'a self) -> PollRecv<'a, Self>
    where
        Self: Unpin,
    {
        PollRecv { t: self }
    }

    fn send_dgram<'a>(&'a self, buf: &'a [u8], addr: SocketAddr) -> PollSend<'a, Self>
    where
        Self: Unpin,
    {
        PollSend { t: self, buf, addr }
    }
}

struct DatagramSocketStream<T>(T);

impl<T: DatagramSocket + Unpin> Stream for DatagramSocketStream<T> {
    type Item = T::RecvType;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let item = ready!(Pin::new(&self.0).poll_recv(cx));
        Poll::Ready(item.ok())
    }
}

impl<T: DatagramSocket + Unpin> Sink<(Bytes, SocketAddr)> for DatagramSocketStream<T> {
    type Error = anyhow::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn start_send(self: Pin<&mut Self>, item: (Bytes, SocketAddr)) -> Result<(), Self::Error> {
        todo!()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

pub struct PollRecv<'a, T: ?Sized> {
    t: &'a T,
}

pub struct PollSend<'a, T: ?Sized> {
    t: &'a T,
    buf: &'a [u8],
    addr: SocketAddr,
}

impl<'a, T: DatagramSocket + Unpin + ?Sized> Future for PollRecv<'a, T> {
    type Output = std::io::Result<T::RecvType>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(self.t).poll_recv(cx)
    }
}

impl<'a, T: DatagramSocket + Unpin + ?Sized> Future for PollSend<'a, T> {
    type Output = std::io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(self.t).poll_send(cx, self.buf, self.addr)
    }
}
