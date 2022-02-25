use crate::buf::{Buf, RWBuffer};
use crate::socks5::Address;
use futures_lite::future::race;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use smol::net::{
    TcpListener as AsyncTcpListener, TcpStream as AsyncTcpStream, UdpSocket as AsyncUdpSocket,
};
use smol::spawn;
use std::borrow::Cow;
use std::io::{IoSlice, IoSliceMut};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::ops::{Deref, DerefMut};
#[cfg(target_os = "linux")]
use std::os::unix::prelude::{AsRawFd, RawFd};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

pub struct UdpSocket(AsyncUdpSocket);

static UDP_SOCKET_COUNT: AtomicUsize = AtomicUsize::new(0);
static TCP_SOCKET_COUNT: AtomicUsize = AtomicUsize::new(0);

impl From<AsyncUdpSocket> for UdpSocket {
    fn from(s: AsyncUdpSocket) -> Self {
        UDP_SOCKET_COUNT.fetch_add(1, Ordering::Acquire);
        Self(s)
    }
}

impl UdpSocket {
    pub fn is_v4(&self) -> bool {
        match self.0.local_addr() {
            Ok(v) => v.is_ipv4(),
            _ => true,
        }
    }

    pub async fn bind(v4: bool) -> smol::io::Result<Self> {
        Self::bind_addr(if v4 {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
        })
        .await
    }

    pub async fn bind_addr(ip: IpAddr) -> smol::io::Result<Self> {
        Ok(UdpSocket::from(
            AsyncUdpSocket::bind(SocketAddr::new(ip, 0)).await?,
        ))
    }

    pub async fn send_to_addr(&self, buf: &[u8], addr: &Address<'_>) -> smol::io::Result<usize> {
        match addr {
            Address::IP(addr) => self.send_to(buf, addr).await,
            Address::Name { host, port } => self.send_to(buf, (host.as_ref(), *port)).await,
        }
    }
}

impl Deref for UdpSocket {
    type Target = AsyncUdpSocket;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for UdpSocket {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        let count = UDP_SOCKET_COUNT.fetch_sub(1, Ordering::Acquire);
        log::debug!("Dropping UDP socket. Remaining: {count}");
    }
}

pub struct TcpStream(AsyncTcpStream);

impl TcpStream {
    pub async fn connect_raw(a: impl smol::net::AsyncToSocketAddrs) -> smol::io::Result<Self> {
        Ok(Self::from(AsyncTcpStream::connect(a).await?))
    }

    pub async fn connect(a: &Address<'_>) -> smol::io::Result<Self> {
        match a {
            Address::IP(addr) => Ok(TcpStream::from(AsyncTcpStream::connect(addr).await?)),
            Address::Name { host, port } => Ok(TcpStream::from(
                AsyncTcpStream::connect((host.as_ref(), *port)).await?,
            )),
        }
    }

    pub fn is_v4(&self) -> bool {
        match self.0.local_addr() {
            Ok(v) => v.is_ipv4(),
            _ => true,
        }
    }
}

impl From<AsyncTcpStream> for TcpStream {
    fn from(s: AsyncTcpStream) -> Self {
        TCP_SOCKET_COUNT.fetch_add(1, Ordering::Acquire);
        Self(s)
    }
}

impl Deref for TcpStream {
    type Target = AsyncTcpStream;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TcpStream {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        let count = TCP_SOCKET_COUNT.fetch_sub(1, Ordering::SeqCst);
        log::debug!("Dropping TCP socket. Remaining: {count}");
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }

    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_read_vectored(cx, bufs)
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_close(cx)
    }
}

pub struct TcpListener(AsyncTcpListener);

impl TcpListener {
    pub fn from(v: AsyncTcpListener) -> Self {
        Self(v)
    }

    pub fn local_addr(&self) -> smol::io::Result<SocketAddr> {
        self.0.local_addr()
    }

    pub async fn bind(addr: &Address<'_>) -> smol::io::Result<Self> {
        let inner = match addr {
            Address::IP(addr) => AsyncTcpListener::bind(addr).await?,
            Address::Name { host, port } => AsyncTcpListener::bind((host.as_ref(), *port)).await?,
        };
        Ok(Self(inner))
    }

    pub async fn accept(&self) -> smol::io::Result<(TcpStream, SocketAddr)> {
        let (stream, addr) = self.0.accept().await?;
        Ok((TcpStream::from(stream), addr))
    }

    #[cfg(target_os = "linux")]
    pub fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

pub async fn copy_udp_and_udp(
    src: UdpSocket,
    mut src_buf: Buf,
    dst: UdpSocket,
    src_hdr_len: Option<usize>,
    dst_hdr_len: Option<usize>,
    src_to_dst_fn: impl Fn(SocketAddr, &mut Buf) -> anyhow::Result<Option<(Address, Cow<[u8]>)>>
        + Send
        + Sync
        + 'static,
    dst_to_src_fn: impl Fn(SocketAddr, &mut Buf) -> anyhow::Result<Option<(Address, Cow<[u8]>)>>
        + Send
        + Sync
        + 'static,
) -> anyhow::Result<()> {
    let src = Arc::new(src);
    let dst = Arc::new(dst);

    let task1 = {
        let src = src.clone();
        let dst = dst.clone();
        spawn(async move {
            let start = dst_hdr_len.unwrap_or(0);
            if start >= src_buf.capacity() {
                panic!("Header size is greater than buf capacity");
            }

            loop {
                src_buf.set_len(src_buf.capacity());
                let addr = match src.recv_from(&mut src_buf[start..]).await? {
                    (n, a) if n > 0 => {
                        src_buf.set_len(start + n);
                        a
                    }
                    _ => return Ok(()),
                };

                if let Some((addr, buf)) = src_to_dst_fn(addr, &mut src_buf)? {
                    dst.send_to_addr(buf.as_ref(), &addr).await?;
                }
            }
        })
    };

    let task2 = {
        let src = src.clone();
        let dst = dst.clone();
        spawn(async move {
            let mut buf = Buf::new_with_len(65536, 65536);
            let start = src_hdr_len.unwrap_or(0);
            if start >= buf.capacity() {
                panic!("Header size is greater than buf capacity");
            }

            loop {
                buf.set_len(buf.capacity());
                let addr = match dst.recv_from(&mut buf[start..]).await? {
                    v if v.0 > 0 => {
                        buf.set_len(start + v.0);
                        v.1
                    }
                    _ => return Ok(()),
                };

                if let Some((addr, buf)) = dst_to_src_fn(addr, &mut buf)? {
                    src.send_to_addr(buf.as_ref(), &addr).await?;
                }
            }
        })
    };

    race(task1, task2).await
}

pub async fn copy_udp_and_stream(
    udp: UdpSocket,
    mut udp_buf: Buf,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    stream_hdr_max_size: Option<usize>,
    transform_udp_buf: impl Fn(SocketAddr, Option<&mut Buf>, &mut Buf) -> anyhow::Result<()>
        + Send
        + Sync
        + 'static,
    transform_stream_buf: impl Fn(&[u8], &mut Buf) -> anyhow::Result<Option<(usize, Address<'static>)>>
        + Send
        + Sync
        + 'static,
) -> anyhow::Result<()> {
    let udp = Arc::new(udp);
    let (mut r, mut w) = split(stream);

    let task1 = {
        let udp = udp.clone();
        spawn(async move {
            let mut stream_hdr = stream_hdr_max_size.map(Buf::new);
            loop {
                udp_buf.set_len(udp_buf.capacity());

                let addr = match udp.recv_from(&mut udp_buf).await? {
                    v if v.0 == 0 => return Ok(()),
                    (n, addr) => {
                        udp_buf.set_len(n);
                        addr
                    }
                };

                transform_udp_buf(addr, stream_hdr.as_mut(), &mut udp_buf)?;
                match stream_hdr.as_ref() {
                    Some(hdr) if !hdr.is_empty() => {
                        w.write_all(&hdr).await?;
                    }
                    _ => {}
                };

                if !udp_buf.is_empty() {
                    w.write_all(&udp_buf).await?;
                }
            }
        })
    };

    let task2 = {
        let udp = udp.clone();
        spawn(async move {
            loop {
                let mut stream_buf = RWBuffer::new(67000, 67000);
                let mut udp_buf = Buf::new(65536);
                match r.read(stream_buf.write_buf()).await? {
                    0 => return Ok(()),
                    v => stream_buf.advance_write(v),
                };

                while stream_buf.remaining_read() > 0 {
                    udp_buf.set_len(0);
                    match transform_stream_buf(stream_buf.read_buf(), &mut udp_buf)? {
                        Some((offset, addr)) => {
                            udp.send_to_addr(&udp_buf, &addr).await?;
                            stream_buf.advance_read(offset);
                        }
                        None => break,
                    }
                }

                if stream_buf.should_compact() {
                    stream_buf.compact();
                }
            }
        })
    };

    race(task1, task2).await
}
