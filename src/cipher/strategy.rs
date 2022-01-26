use super::noop::new_no_op;
use super::partial::new_partial_stream_cipher;
use super::suite::BoxedStreamCipher;
use crate::proxy::protocol::{ProxyRequest, ProxyRequestType};
use anyhow::anyhow;
use std::num::NonZeroUsize;
use std::str::FromStr;

#[derive(Debug)]
pub enum EncryptionStrategy {
    FirstN(NonZeroUsize),
    Always,
    Never,
}

impl std::fmt::Display for EncryptionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FirstN(v) => std::fmt::Display::fmt(v, f),
            Self::Always => f.write_str("a"),
            Self::Never => f.write_str("n"),
        }
    }
}

impl EncryptionStrategy {
    pub fn pick_send(req: &ProxyRequest) -> Self {
        match &req.t {
            ProxyRequestType::SocksTCP(addr) if addr.get_port() == 443 => {
                Self::FirstN(NonZeroUsize::try_from(512).unwrap())
            }
            _ => EncryptionStrategy::Always,
        }
    }

    pub fn pick_receive(req: &ProxyRequest) -> Self {
        match &req.t {
            ProxyRequestType::SocksTCP(addr) if addr.get_port() == 443 => EncryptionStrategy::Never,
            _ => EncryptionStrategy::Always,
        }
    }

    pub fn wrap_cipher(&self, c: BoxedStreamCipher) -> BoxedStreamCipher {
        match self {
            EncryptionStrategy::FirstN(n) => new_partial_stream_cipher(*n, c),
            EncryptionStrategy::Always => c,
            EncryptionStrategy::Never => new_no_op(),
        }
    }
}

impl FromStr for EncryptionStrategy {
    type Err = anyhow::Error;

    fn from_str(v: &str) -> Result<Self, Self::Err> {
        match v.parse::<usize>() {
            Ok(v) if v > 0 => return Ok(Self::FirstN(NonZeroUsize::try_from(v).unwrap())),
            Ok(_) => return Err(anyhow!("Invalid enc strategy {v}")),
            _ => {}
        };

        match v {
            "n" => Ok(Self::Never),
            "a" => Ok(Self::Always),
            _ => return Err(anyhow!("Invalid enc strategy {v}")),
        }
    }
}
