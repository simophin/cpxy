pub mod utils {
    use std::net::SocketAddr;

    use super::super::TransparentUdpSocket;

    struct TestTransparentSocket {
        addr: SocketAddr,
    }

    pub fn bind_transparent_udp(
        addr: SocketAddr,
    ) -> anyhow::Result<impl TransparentUdpSocket + Unpin + Send + Sync> {
        Ok(TestTransparentSocket { addr })
    }

    impl TransparentUdpSocket for TestTransparentSocket {
        fn poll_recv(
            self: std::pin::Pin<&Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut [u8],
        ) -> std::task::Poll<std::io::Result<((usize, SocketAddr), SocketAddr)>> {
            todo!()
        }

        fn poll_send_to(
            self: std::pin::Pin<&Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
            addr: SocketAddr,
        ) -> std::task::Poll<std::io::Result<usize>> {
            todo!()
        }
    }
}
