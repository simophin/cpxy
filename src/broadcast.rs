use std::task::Poll;

use futures_lite::Stream;
use pin_project_lite::pin_project;

pin_project! {
    #[derive(Clone)]
    pub struct Receiver<T> {
        init: Option<T>,
        #[pin]
        rx: async_broadcast::Receiver<T>,
    }
}

impl<T: Clone> Stream for Receiver<T> {
    type Item = T;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.project();
        match (Stream::poll_next(this.rx, cx), this.init.as_ref()) {
            (Poll::Ready(Some(item)), _) => {
                *this.init = None;
                Poll::Ready(Some(item))
            }
            (Poll::Pending, Some(value)) => {
                let value = value.clone();
                *this.init = None;
                Poll::Ready(Some(value))
            }
            (v, _) => v,
        }
    }
}

pub fn bounded<T: Clone>(init: Option<T>, cap: usize) -> (async_broadcast::Sender<T>, Receiver<T>) {
    let (mut tx, rx) = async_broadcast::broadcast::<T>(cap);
    tx.set_overflow(true);
    (tx, Receiver { init, rx })
}

#[cfg(test)]
mod test {
    use super::*;
    use futures_lite::StreamExt;
    use crate::rt::block_on;

    #[test]
    fn test_channel_init() {
        block_on(async move {
            let (tx, mut rx1) = bounded(Some(1), 1);
            let mut rx2 = rx1.clone();
            assert_eq!(Some(1), rx1.next().await);
            assert_eq!(Some(1), rx2.next().await);
            let _ = tx.broadcast(2).await;
            assert_eq!(Some(2), rx1.next().await);
        })
    }
}
