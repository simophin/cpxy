pub mod server;

use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use async_trait::async_trait;
use futures::AsyncWriteExt;
use serde::{Deserialize, Serialize};

use crate::{
    buf::RWBuffer,
    fetch::connect_http_stream,
    http::{parse_response, HttpRequestBuilder},
    io::{connect_tcp, connect_tcp_marked, AsRawFdExt, AsyncStreamCounter},
    proxy::protocol::ProxyRequest,
    socks5::Address,
};

use super::{AsyncStream, BoxedSink, BoxedStream, Protocol, Stats};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpProxy {
    pub address: Address<'static>,
    pub ssl: bool,
}

#[async_trait]
impl Protocol for HttpProxy {
    fn supports(&self, req: &ProxyRequest<'_>) -> bool {
        match req {
            ProxyRequest::HTTP { .. } | ProxyRequest::TCP { .. } => true,
            _ => false,
        }
    }

    async fn new_stream(
        &self,
        dst: &Address<'_>,
        initial_data: Option<&[u8]>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Box<dyn AsyncStream>> {
        let upstream = connect_tcp_marked(&self.address, fwmark)
            .await
            .context("Connecting to HTTP Proxy")?;

        let upstream = connect_http_stream(self.ssl, &self.address, upstream).await?;

        let mut upstream = AsyncStreamCounter::new(upstream, stats.rx.clone(), stats.tx.clone());
        upstream
            .write_all(&HttpRequestBuilder::new("CONNECT", dst)?.finalise())
            .await
            .context("Writing CONNECT request")?;

        match initial_data {
            Some(d) if d.len() > 0 => upstream
                .write_all(d)
                .await
                .context("Writing initial data")?,
            _ => {}
        };

        let upstream = parse_response(upstream, RWBuffer::new_vec_uninitialised(128))
            .await
            .context("Parsing response")?;

        if upstream.status_code != 200 {
            bail!(
                "Invalid status code from HTTP Proxy: {}",
                upstream.status_code
            );
        }

        Ok(Box::new(upstream))
    }

    async fn new_stream_conn(
        &self,
        req: &ProxyRequest<'_>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(Box<dyn AsyncStream>, Duration)> {
        if !self.supports(req) {
            bail!("Unsupported request {req:?}");
        }

        let start = Instant::now();

        let upstream = connect_tcp(&self.address)
            .await
            .context("Connecting to upstream socket")?;

        if let Some(m) = fwmark {
            upstream.set_sock_mark(m)?;
        }

        let upstream = AsyncStreamCounter::new(upstream, stats.rx.clone(), stats.tx.clone());
        let mut upstream = connect_http_stream(self.ssl, &self.address, upstream)
            .await
            .context("Connecting to http")?;

        match req {
            ProxyRequest::TCP { dst } => {
                upstream
                    .write_all(&HttpRequestBuilder::new("CONNECT", dst)?.finalise())
                    .await
                    .context("Writing HTTP CONNECT")?;

                let upstream = parse_response(upstream, RWBuffer::new_vec_uninitialised(512))
                    .await
                    .context("Parsing CONNECT response")?;

                if upstream.status_code != 200 {
                    bail!("Invalid CONNECT response code: {}", upstream.status_code);
                }

                Ok((Box::new(upstream), start.elapsed()))
            }
            ProxyRequest::HTTP { dst, https, req } => {
                let mut url = match (*https, dst.get_port()) {
                    (true, 443) => format!("https://{}", dst.get_host()),
                    (false, 80) => format!("http://{}", dst.get_host()),
                    (true, _) => format!("https://{dst}"),
                    (false, _) => format!("http://{dst}"),
                };

                if req.path.as_ref() != "/" {
                    url.push_str(&req.path);
                }

                let mut builder = HttpRequestBuilder::new(&req.method, url)?;
                for (k, v) in &req.headers {
                    builder.put_header(k, v)?;
                }

                upstream
                    .write_all(&builder.finalise())
                    .await
                    .context("Writing headers to HTTP Proxy")?;

                Ok((Box::new(upstream), start.elapsed()))
            }
            _ => bail!("Unsupported request {req:?}"),
        }
    }

    async fn new_dgram_conn(
        &self,
        _: &ProxyRequest<'_>,
        _: &Stats,
        _: Option<u32>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        bail!("Datagram not supported")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::{
            direct::Direct,
            test::{test_protocol_http, test_protocol_tcp},
        },
        rt::{block_on, spawn},
        test::create_http_server,
        url::HttpUrl,
    };

    #[test]
    fn http_proxy_works() {
        let _ = env_logger::try_init();
        block_on(async move {
            let (server, server_url) = create_http_server().await;
            let url: HttpUrl = server_url.as_str().try_into().unwrap();
            spawn(super::server::serve(server, Direct {})).detach();

            let protocol = HttpProxy {
                address: url.address.clone().into_owned(),
                ssl: url.is_https,
            };

            test_protocol_http(&protocol).await;
            test_protocol_tcp(&protocol).await;
        });
    }
}
