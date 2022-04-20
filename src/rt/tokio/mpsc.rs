use std::{pin::Pin, task::Poll};

use futures_lite::Stream;
pub use tokio::sync::mpsc::{error::TrySendError, Sender};

use tokio::sync::mpsc::{channel as tokio_channel, Receiver as TokioReceiver};

pub struct Receiver<T> {
    inner: TokioReceiver<T>,
}

impl<T> Stream for Receiver<T> {
    type Item = T;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        TokioReceiver::poll_recv(&mut self.inner, cx)
    }
}

pub fn bounded<T>(caps: usize) -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = tokio_channel(caps);
    (tx, Receiver { inner: rx })
}
