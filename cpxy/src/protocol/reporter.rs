use atomic::Atomic;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub trait ProtocolReporter: Send + Sync {
    fn inc_tx(&self, n: usize);
    fn inc_rx(&self, n: usize);
    fn report_delay(&self, delay: Duration);

    fn report(&self) -> ProtocolReport;
    fn last_activity(&self) -> Option<Instant>;
    fn average_delay(&self) -> Duration;
}

impl<R: ProtocolReporter> ProtocolReporter for Arc<R> {
    fn inc_tx(&self, n: usize) {
        self.as_ref().inc_tx(n);
    }

    fn inc_rx(&self, n: usize) {
        self.as_ref().inc_rx(n);
    }

    fn report_delay(&self, delay: Duration) {
        self.as_ref().report_delay(delay);
    }

    fn report(&self) -> ProtocolReport {
        self.as_ref().report()
    }

    fn average_delay(&self) -> Duration {
        self.as_ref().average_delay()
    }

    fn last_activity(&self) -> Option<Instant> {
        self.as_ref().last_activity()
    }
}

#[derive(Default)]
pub struct AtomicProtocolReporter {
    rx: AtomicUsize,
    tx: AtomicUsize,

    delays: Atomic<(Duration, usize)>,
    last_activity: Atomic<Option<Instant>>,
}

impl ProtocolReporter for AtomicProtocolReporter {
    fn inc_tx(&self, n: usize) {
        self.tx.fetch_add(n, Ordering::Relaxed);
    }

    fn inc_rx(&self, n: usize) {
        self.rx.fetch_add(n, Ordering::Relaxed);
    }

    fn report_delay(&self, delay: Duration) {
        self.delays
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |(total, n)| {
                Some((total + delay, n + 1))
            })
            .unwrap();

        self.last_activity
            .store(Some(Instant::now()), Ordering::Relaxed);
    }

    fn average_delay(&self) -> Duration {
        let (total, n) = self.delays.load(Ordering::Relaxed);
        total / (n as u32)
    }

    fn report(&self) -> ProtocolReport {
        ProtocolReport {
            rx: self.rx.load(Ordering::Relaxed),
            tx: self.rx.load(Ordering::Relaxed),
            average_delay_mills: self.average_delay().as_millis() as u64,
            last_activity: self.last_activity().map(|last| {
                let elapsed = last.elapsed();
                Utc::now() - chrono::Duration::from_std(elapsed).unwrap()
            }),
        }
    }

    fn last_activity(&self) -> Option<Instant> {
        self.last_activity.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
pub struct NoopProtocolReporter;

impl ProtocolReporter for NoopProtocolReporter {
    fn inc_tx(&self, _n: usize) {}

    fn inc_rx(&self, _n: usize) {}

    fn report_delay(&self, _delay: Duration) {}

    fn average_delay(&self) -> Duration {
        Duration::from_secs(0)
    }

    fn report(&self) -> ProtocolReport {
        ProtocolReport {
            rx: 0,
            tx: 0,
            average_delay_mills: 0,
            last_activity: None,
        }
    }

    fn last_activity(&self) -> Option<Instant> {
        None
    }
}

#[derive(Serialize, Debug)]
pub struct ProtocolReport {
    pub rx: usize,
    pub tx: usize,
    pub average_delay_mills: u64,
    pub last_activity: Option<DateTime<Utc>>,
}
