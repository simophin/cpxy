mod cipher;
mod dgram;
pub mod server;
mod udp_stream;

use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use async_trait::async_trait;
use futures::AsyncReadExt;
use serde::{Deserialize, Serialize};

use crate::io::{AsyncStreamCounter, TcpStreamExt};
use crate::proxy::protocol::ProxyResult;
use crate::utils::{read_bincode_lengthed_async, write_bincode_lengthed};
use crate::{fetch::connect_http, proxy::protocol::ProxyRequest, socks5::Address, url::HttpUrl};

use self::{
    cipher::strategy::EncryptionStrategy,
    dgram::{create_udp_sink, create_udp_stream},
};

use super::{AsyncStream, BoxedSink, BoxedStream, Protocol, Stats};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TcpMan {
    pub address: Address<'static>,
    pub ssl: bool,
}

impl TcpMan {
    async fn send_request(
        &self,
        req: &ProxyRequest<'_>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(impl AsyncStream, Duration)> {
        let start = Instant::now();

        let stream = connect_http(self.ssl, &self.address)
            .await
            .with_context(|| format!("Connecting to {}", self.address))?;

        if let Some(m) = fwmark {
            stream.inner().set_sock_mark(m)?;
        }

        let mut initial_data = Vec::new();
        write_bincode_lengthed(&mut initial_data, req)?;
        let mut stream = cipher::client::connect(
            &HttpUrl {
                is_https: self.ssl,
                address: self.address.clone(),
                path: Default::default(),
            },
            AsyncStreamCounter::new(stream, stats.rx.clone(), stats.tx.clone()),
            EncryptionStrategy::pick_send(req, self.ssl),
            EncryptionStrategy::pick_receive(req),
            initial_data,
        )
        .await
        .context("Connect encryption")?;

        match read_bincode_lengthed_async(&mut stream)
            .await
            .context("Reading proxy response")?
        {
            ProxyResult::Granted { .. } => Ok((stream, start.elapsed())),
            v => Err(v.into()),
        }
    }
}

#[async_trait]
impl Protocol for TcpMan {
    fn supports(&self, _: &ProxyRequest<'_>) -> bool {
        true
    }

    async fn new_stream_conn(
        &self,
        req: &ProxyRequest<'_>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)> {
        match req {
            ProxyRequest::TCP { .. } | ProxyRequest::HTTP { .. } => {}
            _ => bail!("Invalid request {req:?} for streaming"),
        };

        let (stream, latency) = self.send_request(req, stats, fwmark).await?;
        Ok((Box::new(stream), latency))
    }

    async fn new_dgram_conn(
        &self,
        req: &ProxyRequest<'_>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        if !matches!(req, ProxyRequest::UDP { .. }) {
            bail!("Unsupported request type for dgram conn: {req:?}");
        }

        let (stream, _) = self.send_request(req, stats, fwmark).await?;
        let (r, w) = stream.split();
        Ok((
            Box::pin(create_udp_sink(w)),
            Box::pin(create_udp_stream(r, None)),
        ))
    }
}
