use std::sync::{Arc, Weak};
use std::{
    net::SocketAddr,
    sync::Mutex,
    task::{Poll, Waker},
};

use futures_lite::{AsyncRead, AsyncWrite};
use smol::channel::Sender;

use crate::buf::RWBuffer;

pub struct AsyncTcpSocket {
    pub src: SocketAddr,
    pub dst: SocketAddr,
    tx: Weak<Mutex<(RWBuffer, Option<Waker>)>>,
    rx: Weak<Mutex<(RWBuffer, Option<Waker>)>>,
    close_requested: Arc<Mutex<Option<Waker>>>,
    poll_notification: Sender<()>,
}

impl AsyncTcpSocket {
    pub fn new(
        src: SocketAddr,
        dst: SocketAddr,
        tx: &Arc<Mutex<(RWBuffer, Option<Waker>)>>,
        rx: &Arc<Mutex<(RWBuffer, Option<Waker>)>>,
        close_requested: Arc<Mutex<Option<Waker>>>,
        poll_notification: Sender<()>,
    ) -> Self {
        AsyncTcpSocket {
            src,
            dst,
            tx: Arc::downgrade(tx),
            rx: Arc::downgrade(rx),
            close_requested,
            poll_notification,
        }
    }
}

impl AsyncRead for AsyncTcpSocket {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        if buf.len() == 0 {
            return Poll::Ready(Ok(0));
        }

        let rx = match self.rx.upgrade() {
            Some(v) => v,
            _ => return Poll::Ready(Ok(0)),
        };

        let mut rx = rx.lock().expect("To lock receive mutex");
        let to_read = rx.0.remaining_read().min(buf.len());

        if to_read == 0 {
            rx.1.replace(cx.waker().clone());
            return Poll::Pending;
        }

        (&mut buf[..to_read]).copy_from_slice(&rx.0.read_buf()[..to_read]);
        rx.0.advance_read(to_read);
        if rx.0.remaining_read() > 0 {
            cx.waker().wake_by_ref();
        } else {
            rx.1.replace(cx.waker().clone());
        }

        Poll::Ready(Ok(to_read))
    }
}

impl AsyncWrite for AsyncTcpSocket {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if buf.len() == 0 {
            return Poll::Ready(Ok(0));
        }

        let write_len;
        {
            let tx = match self.tx.upgrade() {
                Some(v) => v,
                _ => return Poll::Ready(Ok(0)),
            };

            let mut tx = tx.lock().expect("To lock tx mutex");
            tx.0.remaining_write().min(buf.len());

            if write_len == 0 {
                tx.1.replace(cx.waker().clone());
                return Poll::Pending;
            }

            (&mut tx.0.write_buf()[..write_len]).copy_from_slice(&buf[..write_len]);
            tx.0.advance_write(write_len);

            if tx.0.remaining_write() > 0 {
                cx.waker().wake_by_ref();
            } else {
                tx.1.replace(cx.waker().clone());
            }
        }

        let _ = self.poll_notification.try_send(());
        Poll::Ready(Ok(write_len))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        let tx = match self.tx.upgrade() {
            Some(v) => v,
            _ => return Poll::Ready(Ok(())),
        };

        let mut tx = tx.lock().expect("To lock tx mutex");
        if tx.0.remaining_write() == 0 {
            Poll::Ready(Ok(()))
        } else {
            tx.1.replace(cx.waker().clone());
            Poll::Pending
        }
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match Self::poll_flush(self, cx) {
            Poll::Ready(_) => {}
            Poll::Pending => return Poll::Pending,
        };

        if self.tx.upgrade().is_none() {
            return Poll::Ready(Ok(()));
        }

        let mut requested = self
            .close_requested
            .lock()
            .expect("To lock close requested");
        requested.replace(cx.waker().clone());
        Poll::Ready(Ok(()))
    }
}
