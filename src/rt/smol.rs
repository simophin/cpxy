pub use smol::{block_on, spawn, Executor, Task, Timer};

pub mod net {
    use bytes::Bytes;
    use std::{
        io::ErrorKind,
        net::SocketAddr,
        pin::Pin,
        sync::Arc,
        task::{Context, Poll},
    };

    use crate::utils::{new_vec_for_udp, VecExt};
    use derive_more::Deref;
    pub use smol::net::{AsyncToSocketAddrs, TcpListener, TcpStream};
    use smol::Async;

    pub async fn resolve<A: AsyncToSocketAddrs>(
        name: A,
    ) -> std::io::Result<impl Iterator<Item = SocketAddr>> {
        Ok(smol::net::resolve(name).await?.into_iter())
    }

    #[derive(Deref)]
    pub struct UdpSocket {
        #[deref]
        inner: smol::net::UdpSocket,
        s: Arc<Async<std::net::UdpSocket>>,
    }

    impl UdpSocket {
        pub async fn bind(addr: impl AsyncToSocketAddrs) -> std::io::Result<Self> {
            let inner = smol::net::UdpSocket::bind(addr).await?;
            Ok(Self {
                s: inner.clone().into(),
                inner,
            })
        }

        pub async fn read_with<R>(
            &self,
            op: impl FnMut(&std::net::UdpSocket) -> std::io::Result<R>,
        ) -> std::io::Result<R> {
            self.s.read_with(op).await
        }

        #[inline]
        pub fn poll_readable(self: Pin<&Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            self.s.poll_readable(cx)
        }

        #[inline]
        pub fn poll_writable(self: Pin<&Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            self.s.poll_writable(cx)
        }

        pub async fn recv_bytes_from(&self) -> std::io::Result<(Bytes, SocketAddr)> {
            let mut buf = new_vec_for_udp();
            let (len, addr) = self.recv_from(&mut buf).await?;
            buf.set_len_uninit(len);
            Ok((buf.into(), addr))
        }

        pub fn try_recv_from(
            &self,
            buf: &mut [u8],
        ) -> std::io::Result<Option<(usize, SocketAddr)>> {
            match self.s.get_ref().recv_from(buf) {
                Ok(v) => Ok(Some(v)),
                Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(None),
                Err(e) => Err(e),
            }
        }

        pub fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<Option<usize>> {
            match self.s.get_ref().send_to(buf, addr) {
                Ok(v) => Ok(Some(v)),
                Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(None),
                Err(e) => Err(e),
            }
        }
    }

    impl TryFrom<std::net::UdpSocket> for UdpSocket {
        type Error = std::io::Error;

        fn try_from(value: std::net::UdpSocket) -> Result<Self, Self::Error> {
            let inner: smol::net::UdpSocket = value.try_into()?;
            Ok(Self {
                s: inner.clone().into(),
                inner,
            })
        }
    }
}

pub mod mpsc {
    pub use futures::channel::mpsc::*;
}

pub mod fs {
    pub use smol::fs::{create_dir_all, File};
}

pub use smol_timeout::TimeoutExt;
