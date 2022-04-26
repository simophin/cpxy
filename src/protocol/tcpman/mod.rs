mod cipher;

use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{fetch::connect_http, proxy::protocol::ProxyRequest, socks5::Address, url::HttpUrl};

use self::cipher::{client::connect, strategy::EncryptionStrategy};

use super::{AsyncDgram, AsyncStream, Protocol};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TcpMan {
    address: Address<'static>,
    ssl: bool,
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
        }

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
        Ok((Box::new(stream), duration))
    }

    async fn new_dgram_conn(&self, req: &ProxyRequest<'_>) -> anyhow::Result<Box<dyn AsyncDgram>> {
        match req {
            ProxyRequest::TCP { dst } => todo!(),
            ProxyRequest::UDP {
                initial_dst,
                initial_data,
            } => todo!(),
            ProxyRequest::DNS { domains } => todo!(),
            ProxyRequest::HTTP { dst, https, req } => todo!(),
            ProxyRequest::EchoTestTcp => todo!(),
            ProxyRequest::EchoTestUdp => todo!(),
        }
    }
}
