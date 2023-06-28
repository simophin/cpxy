use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::io::{connect_tcp, AsRawFdExt, CounterStream};
use crate::protocol::ProxyRequest;
use crate::tls::TlsStream;

use super::{Protocol, Stats};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Direct;

#[async_trait]
impl Protocol for Direct {
    type Stream = TlsStream<CounterStream<TcpStream>>;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::Stream> {
        let stream = connect_tcp(&req.dst).await?;
        if let Some(fwmark) = fwmark {
            stream.set_sock_mark(fwmark)?;
        }

        let stream = CounterStream::new(stream, stats.rx.clone(), stats.tx.clone());
        let mut stream = if req.tls {
            TlsStream::connect_tls(req.dst.get_host().as_ref(), stream).await?
        } else {
            TlsStream::connect_plain(stream).await?
        };

        match &req.initial_data {
            Some(b) if b.len() > 0 => {
                stream.write_all(b).await.context("Writing initial data")?;
            }
            _ => {}
        };

        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test;
    use super::*;

    #[tokio::test]
    async fn test_direct_works() {
        let direct = Direct {};
        test::test_protocol_tcp(&direct).await;
        test::test_protocol_http(&direct).await;
    }
}
