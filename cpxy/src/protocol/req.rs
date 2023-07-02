use crate::addr::Address;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ProxyRequest {
    pub dst: Address,
    pub initial_data: Option<Bytes>,
}

impl From<Address> for ProxyRequest {
    fn from(dst: Address) -> Self {
        Self {
            dst,
            initial_data: None,
        }
    }
}
