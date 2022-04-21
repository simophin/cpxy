use std::{
    collections::HashMap,
    io::ErrorKind,
    net::SocketAddr,
    sync::{Arc, Mutex},
    task::Poll,
};

use bytes::Bytes;
use smol::stream::StreamExt;

use super::TransparentUdpSocket;
use crate::rt::mpsc::{Receiver, Sender, TrySendError};

struct TestTransparentSocket {
    bound: SocketAddr,
    rx: Arc<Mutex<Receiver<std::io::Result<(Bytes, SocketAddr, SocketAddr)>>>>,
    tx: Sender<(Bytes, SocketAddr)>,
}

struct TestEnvironment {
    tsockets: HashMap<SocketAddr, Sender<(Bytes, SocketAddr, SocketAddr)>>,
}

impl TestEnvironment {
    pub async fn send(
        &self,
        buf: &[u8],
        src: SocketAddr,
        dst: SocketAddr,
        orig_dst: SocketAddr,
    ) -> anyhow::Result<()> {
        todo!()
    }
}

impl TransparentUdpSocket for TestTransparentSocket {
    fn poll_recv(
        self: std::pin::Pin<&Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<((usize, SocketAddr), SocketAddr)>> {
        match self.rx.lock().unwrap().poll_next(cx) {
            Poll::Ready(Some(Ok((data, src, dst)))) => {
                let len = buf.len().min(data.len());
                (&mut buf[..len]).copy_from_slice(&data[..len]);
                Poll::Ready(Ok(((len, src), dst)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(e)),
            Poll::Ready(None) => {
                Poll::Ready(Err(std::io::Error::new(ErrorKind::UnexpectedEof, "EOF")))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_send_to(
        self: std::pin::Pin<&Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
        addr: SocketAddr,
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.tx.try_send((Bytes::copy_from_slice(buf), addr)) {
            Ok(_) => Poll::Ready(Ok(buf.len())),
            Err(TrySendError::Closed(_)) => Poll::Ready(Ok(0)),
            Err(TrySendError::Full(_)) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}
