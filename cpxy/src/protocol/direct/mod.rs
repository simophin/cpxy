use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use super::{BoxProtocolReporter, ProxyRequest};
use crate::io::{connect_tcp, AsRawFdExt, CounterStream};
use crate::tls::TlsStream;

use super::Protocol;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Default)]
pub struct Direct;

#[async_trait]
impl Protocol for Direct {
    type ClientStream = TlsStream<CounterStream<TcpStream>>;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        reporter: &BoxProtocolReporter,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::ClientStream> {
        let stream = connect_tcp(&req.dst).await?;
        if let Some(fwmark) = fwmark {
            stream.set_sock_mark(fwmark)?;
        }

        let stream = CounterStream::new(stream, reporter.clone());
        let mut stream = TlsStream::connect_plain(stream).await?;

        match &req.initial_data {
            Some(b) if b.len() > 0 => {
                stream.write_all(b).await.context("Writing initial data")?;
            }
            _ => {}
        };

        Ok(stream)
    }
}

// #[cfg(test)]
// mod tests {
//     use super::super::test;
//     use super::*;
//
//     #[tokio::test]
//     async fn test_direct_works() {
//         let direct = Direct {};
//         test::test_protocol_tcp(&direct).await;
//         test::test_protocol_http(&direct).await;
//     }
// }
