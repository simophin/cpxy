use std::collections::{HashSet, VecDeque};

use atomic::Atomic;
use chrono::{DateTime, Utc};
use lazy_static::lazy_static;
use parking_lot::RwLock;
use serde::Serialize;

#[derive(Serialize)]
struct LogEntry {
    category: &'static str,
    seq: u64,
    time: DateTime<Utc>,
    msg: Box<str>,
}

#[derive(Default)]
struct LogBuffer {
    next_seq: Atomic<u64>,
    queue: RwLock<VecDeque<LogEntry>>,
}

struct LogBufferRead {
    buf: &'static LogBuffer,
    start_seq_exclusive: Option<u64>,
    categories: Option<HashSet<String>>,
    earliest_exclusive: Option<DateTime<Utc>>,
}

fn log_buffer() -> &'static LogBuffer {
    lazy_static! {
        static ref BUFFER: LogBuffer = Default::default();
    }
    &BUFFER
}

pub fn read_log_buffer(
    start_seq_exclusive: Option<u64>,
    categories: Option<HashSet<String>>,
    earliest_exclusive: Option<DateTime<Utc>>,
) -> impl Serialize + 'static {
    LogBufferRead {
        buf: log_buffer(),
        start_seq_exclusive,
        categories,
        earliest_exclusive,
    }
}

const MAX_LOG_ENTRY: usize = 10000;

pub fn print(category: &'static str, args: impl ToString) {
    let msg = args.to_string().into_boxed_str();
    let seq = log_buffer().next_seq.fetch_add(1, atomic::Ordering::AcqRel);
    let mut guard = log_buffer().queue.write();
    guard.push_back(LogEntry {
        category,
        seq,
        time: std::time::SystemTime::now().into(),
        msg,
    });
    while guard.len() > MAX_LOG_ENTRY {
        guard.pop_front();
    }
}

impl Serialize for LogBufferRead {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_seq(self.buf.queue.read().iter().filter(
            |LogEntry {
                 category,
                 seq,
                 time,
                 ..
             }| {
                match &self.categories {
                    Some(values) if !values.contains(*category) => return false,
                    _ => {}
                }

                match &self.start_seq_exclusive {
                    Some(s) if seq <= s => return false,
                    _ => {}
                }

                match &self.earliest_exclusive {
                    Some(d) if time <= d => return false,
                    _ => {}
                }

                true
            },
        ))
    }
}
