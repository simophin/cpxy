use serde::{Deserialize, Serialize};
use std::fmt::Debug;

pub struct BoxBuf(Box<dyn AsRef<[u8]>>);

impl BoxBuf {
    pub fn new(buf: impl AsRef<[u8]> + 'static) -> Self {
        BoxBuf(Box::new(buf))
    }
}

impl Serialize for BoxBuf {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let buf: &[u8] = self.0.as_ref().as_ref();
        buf.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BoxBuf {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let buf = <Vec<u8> as Deserialize>::deserialize(deserializer)?;
        Ok(BoxBuf(Box::new(buf)))
    }
}

impl Debug for BoxBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "BoxBuf(len={})",
            self.0.as_ref().as_ref().len()
        ))
    }
}
