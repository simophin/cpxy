pub use smol::{block_on, spawn, Executor, Task, Timer};

pub mod net {
    use std::sync::Arc;

    use derive_more::Deref;
    pub use smol::net::{resolve, AsyncToSocketAddrs, TcpListener, TcpStream};
    use smol::Async;

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
    pub use smol::channel::*;
}

pub mod fs {
    pub use smol::fs::{create_dir_all, File};
}

pub use smol_timeout::TimeoutExt;
