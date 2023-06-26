pub mod server;

use anyhow::{bail, Context};
use async_trait::async_trait;
use futures::AsyncWriteExt;
use serde::{Deserialize, Serialize};
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::{
    buf::RWBuffer,
    fetch::connect_http_stream,
    http::{parse_response, HttpRequestBuilder},
    io::{connect_tcp_marked, AsyncStreamCounter},
    socks5::Address,
};

use super::{AsyncStream, Protocol, Stats};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpProxy {
    pub address: Address<'static>,
    pub ssl: bool,
    pub auth_header: Option<String>,
}

#[async_trait]
impl Protocol for HttpProxy {
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

        let upstream = connect_http_stream(self.ssl, &self.address, upstream.compat()).await?;

        let mut upstream = AsyncStreamCounter::new(upstream, stats.rx.clone(), stats.tx.clone());

        let mut request = HttpRequestBuilder::new("CONNECT", dst)?;
        if let Some(auth_header) = &self.auth_header {
            request.put_header_text("Proxy-Authorization", auth_header)?;
        }

        upstream
            .write_all(&request.finalise())
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
}

#[cfg(test)]
mod tests {
    use async_shutdown::Shutdown;
    use tokio::spawn;

    use super::*;
    use crate::{
        protocol::{
            direct::Direct,
            test::{test_protocol_http, test_protocol_tcp},
        },
        test::create_http_server,
        url::HttpUrl,
    };

    #[tokio::test]
    async fn http_proxy_works() {
        let _ = env_logger::try_init();
        let (server, server_url) = create_http_server().await;
        let url: HttpUrl = server_url.as_str().try_into().unwrap();
        spawn(server::serve(Shutdown::new(), server, Direct {}));

        let protocol = HttpProxy {
            address: url.address.clone().into_owned(),
            ssl: url.is_https,
            auth_header: None,
        };

        test_protocol_http(&protocol).await;
        test_protocol_tcp(&protocol).await;
    }
}
