mod auth;
pub mod server;

use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::{BoxProtocolReporter, Protocol, ProxyRequest};
use crate::http::parse_response;
use crate::tls::TlsStream;
use crate::{
    addr::Address,
    io::{connect_tcp_marked, CounterStream},
};

use crate::io::time_future;
use std::io::Write;

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
        reporter: &BoxProtocolReporter,
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
        let mut request = Vec::new();
        let host = req.dst.host();
        let port = req.dst.port();
        write!(request, "CONNECT {host}:{port}\r\n")?;
        write!(request, "Host: {host}:{port}\r\n")?;
        if let Some(auth_header) = &self.auth_settings {
            write!(
                request,
                "Proxy-Authorization: {}\r\n",
                auth_header.to_header_value()
            )?;
        }
        request.extend_from_slice(b"\r\n");

        upstream
            .write_all(&request)
            .await
            .context("Tunnel request")?;

        let mut upstream = BufReader::new(upstream);

        // Wait for the response
        parse_response(&mut upstream, |res| {
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
    // use crate::{
    //     protocol::test::{test_protocol_http, test_protocol_tcp},
    //     test::create_http_server,
    // };

    #[tokio::test]
    async fn http_proxy_works() {
        // let _ = env_logger::try_init();
        // let (server, server_url) = create_http_server().await;
        // let url: HttpUrl = server_url.as_str().try_into().unwrap();
        // spawn(server::serve(Shutdown::new(), server, Direct {}));
        //
        // let protocol = HttpProxy {
        //     address: url.address.clone().into_owned(),
        //     ssl: url.is_https,
        //     auth_header: None,
        // };
        //
        // test_protocol_http(&protocol).await;
        // test_protocol_tcp(&protocol).await;
    }
}
