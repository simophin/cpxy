use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default)]
pub struct Counter(AtomicUsize);

impl Counter {
    pub fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }

    pub fn set(&self, value: usize) {
        self.0.store(value, Ordering::Relaxed);
    }

    pub fn inc(&self, value: usize) {
        self.0.fetch_add(value, Ordering::Acquire);
    }
}

impl<'de> Deserialize<'de> for Counter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        u32::deserialize(deserializer).map(|v| Counter(AtomicUsize::new(v as usize)))
    }
}

impl Serialize for Counter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u64(self.get() as u64)
    }
}
