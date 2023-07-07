pub mod auth;
pub mod server;

use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::BufReader;
use tokio::net::TcpStream;

use super::{Protocol, ProxyRequest};
use crate::http::parse_response;
use crate::tls::TlsStream;
use crate::{
    addr::Address,
    io::{connect_tcp_marked, CounterStream},
};

use crate::http::writer::RequestWriter;
use crate::io::time_future;
use crate::protocol::ProtocolReporter;
use hyper::{header, Method};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpProxy {
    pub address: Address,
    pub ssl: bool,
    pub auth_settings: Option<auth::BasicAuthSettings>,
}

#[async_trait]
impl Protocol for HttpProxy {
    type ClientStream = BufReader<TlsStream<CounterStream<TcpStream>>>;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        reporter: &Arc<dyn ProtocolReporter>,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::ClientStream> {
        let (upstream, delay) = time_future(connect_tcp_marked(&self.address, fwmark))
            .await
            .context("Connecting to HTTP Proxy")?;
        reporter.report_delay(delay);

        let upstream = CounterStream::new(upstream, reporter.clone());

        // Establish TLS if required
        let mut upstream = if self.ssl {
            TlsStream::connect_tls(self.address.host(), upstream).await
        } else {
            TlsStream::connect_plain(upstream).await
        }
        .context("Connecting to TLS")?;

        // Establish HTTP tunnel
        let mut writer = RequestWriter::write(Method::CONNECT, &req.dst);

        if let Some(auth_header) = &self.auth_settings {
            writer.write_header(header::PROXY_AUTHORIZATION, auth_header.to_header_value());
        }

        writer
            .to_async(&mut upstream)
            .await
            .context("Writing tunnel request")?;

        let mut upstream = BufReader::new(upstream);

        // Wait for the response
        parse_response(&mut upstream, |res| {
            log::debug!("Got http proxy response: {res:?}");
            let status = res.code.context("Missing status code")?;
            if status < 200 || status >= 300 {
                bail!("Invalid status code from HTTP Proxy: {status}");
            }
            Ok(())
        })
        .await?;

        Ok(upstream)
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::test;
    use std::sync::Arc;

    #[tokio::test]
    async fn http_proxy_works() {
        let _ = env_logger::try_init();

        test::test_protocol_valid_config(
            |addr| super::HttpProxy {
                address: addr.into(),
                ssl: false,
                auth_settings: None,
            },
            Some(super::server::HttpProxyAcceptor {
                auth_provider: None,
            }),
        )
        .await
        .expect("http proxy works");
    }

    #[tokio::test]
    async fn http_proxy_authed_works() {
        let _ = env_logger::try_init();

        let settings = super::auth::BasicAuthSettings {
            user: "test".into(),
            password: "test".into(),
        };

        test::test_protocol_valid_config(
            {
                let settings = settings.clone();
                move |addr| super::HttpProxy {
                    address: addr.into(),
                    ssl: false,
                    auth_settings: Some(settings),
                }
            },
            Some(super::server::HttpProxyAcceptor {
                auth_provider: Some(Arc::new(super::auth::BasicAuthProvider(settings))),
            }),
        )
        .await
        .expect("http proxy works");
    }
}
