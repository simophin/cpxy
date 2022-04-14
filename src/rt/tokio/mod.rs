use std::{pin::Pin, task::Poll, time::Duration};

use derive_more::Deref;
use futures_lite::Future;
use pin_project_lite::pin_project;

pub fn spawn<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> Task<T> {
    Task(tokio::spawn(fut))
}

#[derive(Deref)]
pub struct Task<T>(tokio::task::JoinHandle<T>);

impl<T> Future for Task<T> {
    type Output = T;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match Future::poll(Pin::new(&mut self.0), cx) {
            Poll::Ready(Err(e)) => panic!("Error polling Task: {e:?}"),
            Poll::Ready(Ok(v)) => Poll::Ready(v),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T> Task<T> {
    pub fn detach(&self) {}

    pub fn cancel(&self) {
        self.0.abort()
    }
}

pub fn block_on<T>(f: impl Future<Output = T>) -> T {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(f)
}

pub mod net;

pub mod fs;

pub mod mpsc;

pin_project! {
    pub struct Timeout<T: Future> {
        #[pin]
        inner: tokio::time::Timeout<T>,
    }
}

impl<T: Future> Future for Timeout<T> {
    type Output = Option<T::Output>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        match Future::poll(self.project().inner, cx) {
            Poll::Ready(Ok(k)) => Poll::Ready(Some(k)),
            Poll::Ready(Err(_)) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub trait TimeoutExt: Future + Sized {
    fn timeout(self, after: Duration) -> Timeout<Self>;
}

impl<T: Future + Sized> TimeoutExt for T {
    fn timeout(self, after: Duration) -> Timeout<Self> {
        Timeout {
            inner: tokio::time::timeout(after, self),
        }
    }
}
