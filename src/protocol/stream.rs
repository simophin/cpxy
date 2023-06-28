use super::direct;
use super::http;
use super::socks5;
use super::Protocol;
use std::io::{Error, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub enum ProtocolStream {
    Direct(<direct::Direct as Protocol>::Stream),
    Http(<http::HttpProxy as Protocol>::Stream),
    Socks5(<socks5::Socks5 as Protocol>::Stream),
}

macro_rules! delegate_stream_method {
    ($self:ident, $method:ident, $($p:ident),*) => {
        match $self.get_mut() {
            ProtocolStream::Direct(s) => Pin::new(s).$method($($p),*),
            ProtocolStream::Http(s) => Pin::new(s).$method($($p),*),
            ProtocolStream::Socks5(s) => Pin::new(s).$method($($p),*),
        }
    };
}

impl AsyncRead for ProtocolStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        delegate_stream_method!(self, poll_read, cx, buf)
    }
}

impl AsyncWrite for ProtocolStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        delegate_stream_method!(self, poll_write, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        delegate_stream_method!(self, poll_flush, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        delegate_stream_method!(self, poll_shutdown, cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize, Error>> {
        delegate_stream_method!(self, poll_write_vectored, cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            ProtocolStream::Direct(s) => s.is_write_vectored(),
            ProtocolStream::Http(s) => s.is_write_vectored(),
            ProtocolStream::Socks5(s) => s.is_write_vectored(),
        }
    }
}
