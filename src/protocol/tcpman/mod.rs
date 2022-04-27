mod cipher;
mod dgram;
pub mod server;
mod udp_stream;

use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use async_trait::async_trait;
use futures::AsyncReadExt;
use serde::{Deserialize, Serialize};

use crate::{fetch::connect_http, proxy::protocol::ProxyRequest, socks5::Address, url::HttpUrl};

use self::{
    cipher::{client::connect, strategy::EncryptionStrategy},
    dgram::{create_udp_sink, create_udp_stream},
};

use super::{AsyncStream, BoxedSink, BoxedStream, Protocol};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TcpMan {
    pub address: Address<'static>,
    pub ssl: bool,
}

impl TcpMan {
    async fn send_request(
        &self,
        req: &ProxyRequest<'_>,
    ) -> anyhow::Result<(impl AsyncStream, Duration)> {
        let start = Instant::now();

        let stream = connect_http(self.ssl, &self.address)
            .await
            .with_context(|| format!("Connecting to {}", self.address))?;

        let stream = connect(
            &HttpUrl {
                is_https: self.ssl,
                address: self.address.clone(),
                path: Default::default(),
            },
            stream,
            EncryptionStrategy::pick_send(req, self.ssl),
            EncryptionStrategy::pick_receive(req),
            serde_json::to_vec(req).context("Serialising proxy req")?,
        )
        .await
        .context("Connect encryption")?;

        let duration = start.elapsed();
        Ok((stream, duration))
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
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)> {
        match req {
            ProxyRequest::TCP { .. } | ProxyRequest::HTTP { .. } => {}
            _ => bail!("Invalid request {req:?} for streaming"),
        };

        let (stream, latency) = self.send_request(req).await?;
        Ok((Box::new(stream), latency))
    }

    async fn new_dgram_conn(
        &self,
        req: &ProxyRequest<'_>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        if !matches!(req, ProxyRequest::UDP { .. }) {
            bail!("Unsupported request type for dgram conn: {req:?}");
        }

        let (stream, _) = self.send_request(req).await?;
        let (r, w) = stream.split();
        Ok((Box::new(create_udp_sink(w)), Box::new(create_udp_stream(r))))
    }
}
