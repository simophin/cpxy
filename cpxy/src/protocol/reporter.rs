use atomic::Atomic;
use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
pub struct ProtocolReporter {
    rx: AtomicUsize,
    tx: AtomicUsize,

    delays: Atomic<(Duration, usize)>,
}

pub type BoxProtocolReporter = Arc<ProtocolReporter>;

#[derive(Serialize, Debug)]
pub struct ProtocolReport {
    pub rx: usize,
    pub tx: usize,
    pub average_delay_mills: u64,
}

impl ProtocolReporter {
    pub fn inc_tx(&self, n: usize) {
        self.tx.fetch_add(n, Ordering::Relaxed);
    }

    pub fn inc_rx(&self, n: usize) {
        self.rx.fetch_add(n, Ordering::Relaxed);
    }

    pub fn report_delay(&self, delay: Duration) {
        self.delays
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |(total, n)| {
                Some((total + delay, n + 1))
            })
            .unwrap();
    }

    pub fn average_delay(&self) -> Duration {
        let (total, n) = self.delays.load(Ordering::Relaxed);
        total / (n as u32)
    }

    pub fn report(&self) -> ProtocolReport {
        ProtocolReport {
            rx: self.rx.load(Ordering::Relaxed),
            tx: self.rx.load(Ordering::Relaxed),
            average_delay_mills: self.average_delay().as_millis() as u64,
        }
    }
}
