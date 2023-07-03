use crate::addr::Address;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
use std::str::FromStr;

#[derive(Deserialize, Serialize, Clone)]
pub struct ProxyRequest {
    pub dst: Address,
    pub initial_data: Option<Bytes>,
}

impl Debug for ProxyRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ProxyRequest(dst=[tcp://{}], initial_data=[{} bytes])",
            self.dst,
            self.initial_data
                .as_ref()
                .map(Bytes::len)
                .unwrap_or_default()
        )
    }
}

impl FromStr for ProxyRequest {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let dst = s.parse()?;
        Ok(Self {
            dst,
            initial_data: None,
        })
    }
}

impl From<Address> for ProxyRequest {
    fn from(dst: Address) -> Self {
        Self {
            dst,
            initial_data: None,
        }
    }
}
