pub use smol::{block_on, spawn, Executor, Task, Timer};

pub mod net {
    use bytes::Bytes;

    use std::{
        io::ErrorKind,
        net::SocketAddr,
        pin::Pin,
        sync::{atomic::AtomicUsize, Arc},
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

    static STD_INSTANCE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static NON_STD_INSTANCE_COUNT: AtomicUsize = AtomicUsize::new(0);

    #[derive(Deref)]
    pub struct UdpSocket {
        #[deref]
        inner: smol::net::UdpSocket,
        s: Arc<Async<std::net::UdpSocket>>,
        from_std: bool,
    }

    #[cfg(test)]
    fn should_show_instance_stats() -> bool {
        true
    }

    #[cfg(not(test))]
    fn should_show_instance_stats() -> bool {
        use lazy_static::lazy_static;
        use parking_lot::Mutex;
        use std::time::{Duration, Instant};
        lazy_static! {
            static ref LAST_SHOW: Mutex<Option<Instant>> = Mutex::new(None);
        }

        let mut guard = LAST_SHOW.lock();
        match guard.as_ref() {
            Some(v) if v.elapsed() < Duration::from_secs(10) => false,
            _ => {
                guard.replace(Instant::now());
                true
            }
        }
    }

    fn print_instant_stats() {
        if should_show_instance_stats() {
            log::info!(
                "UDPSocket std counts: {}, non-std counts: {}",
                STD_INSTANCE_COUNT.load(std::sync::atomic::Ordering::Relaxed),
                NON_STD_INSTANCE_COUNT.load(std::sync::atomic::Ordering::Relaxed)
            )
        }
    }

    impl Drop for UdpSocket {
        fn drop(&mut self) {
            if self.from_std {
                log::info!("UDPSocket dropped");
                STD_INSTANCE_COUNT.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            } else {
                NON_STD_INSTANCE_COUNT.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            }

            print_instant_stats();
        }
    }

    impl UdpSocket {
        pub async fn bind(addr: impl AsyncToSocketAddrs) -> std::io::Result<Self> {
            let inner = smol::net::UdpSocket::bind(addr).await?;
            NON_STD_INSTANCE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Acquire);
            print_instant_stats();

            Ok(Self {
                s: inner.clone().into(),
                inner,
                from_std: false,
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
            STD_INSTANCE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Acquire);
            print_instant_stats();
            Ok(Self {
                s: inner.clone().into(),
                inner,
                from_std: true,
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
