use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{de::Visitor, Deserialize, Serialize};

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

struct CounterVisitor;

impl<'de> Visitor<'de> for CounterVisitor {
    type Value = Counter;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a counter value")
    }

    fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Counter(AtomicUsize::new(v.try_into().map_err(|e| {
            serde::de::Error::custom(format!("Error converting number to counter: {e}"))
        })?)))
    }
}

impl<'de> Deserialize<'de> for Counter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_u32(CounterVisitor)
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
