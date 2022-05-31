use std::{ops::DerefMut, pin::Pin, sync::Arc, task::Poll, time::Duration};

use futures::{ready, Future};
use parking_lot::Mutex;

use async_io::Timer as RealTimer;

#[derive(Clone)]
pub struct Timer {
    delay: Duration,
    t: Arc<Mutex<RealTimer>>,
}

impl Timer {
    pub fn new(d: Duration) -> Self {
        Timer {
            delay: d,
            t: Arc::new(Mutex::new(RealTimer::after(d))),
        }
    }

    pub fn reset(&self) {
        *self.t.lock() = RealTimer::after(self.delay);
    }
}

impl Future for Timer {
    type Output = ();
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let mut g = self.t.lock();
        let _ = ready!(Pin::new(g.deref_mut()).poll(cx));
        Poll::Ready(())
    }
}
