use super::partial::PartialStreamCipher;
use super::suite::StreamCipherExt;
use anyhow::anyhow;
use cipher::StreamCipher;
use std::num::NonZeroUsize;
use std::str::FromStr;

#[derive(Debug, Clone, Copy)]
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
    pub fn new_send(is_tcp: bool, dst_port: u16, is_upstream_tls: bool) -> Self {
        match (is_tcp, dst_port, is_upstream_tls) {
            (_, _, true) => Self::Never,
            (true, 443, _) => Self::FirstN(512.try_into().unwrap()),
            _ => Self::Always,
        }
    }

    pub fn new_receive(is_tcp: bool, dst_port: u16) -> Self {
        match (is_tcp, dst_port) {
            (true, 443) => Self::Never,
            _ => Self::Always,
        }
    }

    pub fn wrap_cipher(
        &self,
        c: impl StreamCipherExt + Send + Sync,
    ) -> impl StreamCipherExt + Send + Sync {
        match self {
            EncryptionStrategy::FirstN(n) => {
                StrategyCipher::FirstN(PartialStreamCipher::new(*n, c))
            }
            EncryptionStrategy::Always => StrategyCipher::Always(c),
            EncryptionStrategy::Never => StrategyCipher::Never,
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

enum StrategyCipher<T> {
    FirstN(PartialStreamCipher<T>),
    Always(T),
    Never,
}

impl<T: StreamCipherExt + Send + Sync> StreamCipher for StrategyCipher<T> {
    fn try_apply_keystream_inout(
        &mut self,
        buf: cipher::inout::InOutBuf<'_, '_, u8>,
    ) -> Result<(), cipher::StreamCipherError> {
        match self {
            StrategyCipher::FirstN(stream) => stream.try_apply_keystream_inout(buf),
            StrategyCipher::Always(stream) => stream.try_apply_keystream_inout(buf),
            StrategyCipher::Never => Ok(()),
        }
    }
}

impl<T: StreamCipherExt + Send + Sync> StreamCipherExt for StrategyCipher<T> {
    fn will_modify_data(&self) -> bool {
        match self {
            StrategyCipher::FirstN(stream) => stream.will_modify_data(),
            StrategyCipher::Always(stream) => stream.will_modify_data(),
            StrategyCipher::Never => false,
        }
    }

    fn rewind(&mut self, cnt: usize) {
        match self {
            StrategyCipher::FirstN(stream) => stream.rewind(cnt),
            StrategyCipher::Always(stream) => stream.rewind(cnt),
            StrategyCipher::Never => {}
        }
    }
}
