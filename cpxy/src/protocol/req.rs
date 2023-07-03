use crate::addr::Address;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ProxyRequest {
    pub dst: Address,
    pub initial_data: Option<Bytes>,
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
