use super::{Protocol, Stats};
use crate::io::{connect_tcp, AsRawFdExt, AsyncStreamCounter};
use crate::protocol::AsyncStream;
use crate::socks5::Address;
use anyhow::Context;
use async_trait::async_trait;
use futures_util::AsyncWriteExt;
use serde::{Deserialize, Serialize};
use tokio_util::compat::TokioAsyncReadCompatExt;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Direct;

#[async_trait]
impl Protocol for Direct {
    async fn new_stream(
        &self,
        dst: &Address<'_>,
        initial_data: Option<&[u8]>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Box<dyn AsyncStream>> {
        let stream = connect_tcp(dst).await?;
        if let Some(fwmark) = fwmark {
            stream.set_sock_mark(fwmark)?;
        }

        let mut stream = stream.compat();

        match initial_data {
            Some(b) if b.len() > 0 => {
                stream.write_all(b).await.context("Writing initial data")?;
            }
            _ => {}
        };

        Ok(Box::new(AsyncStreamCounter::new(
            stream,
            stats.rx.clone(),
            stats.tx.clone(),
        )))
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
