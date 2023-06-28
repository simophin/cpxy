pub mod server;

use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::http::parse_response;
use crate::protocol::ProxyRequest;
use crate::tls::TlsStream;
use crate::{
    io::{connect_tcp_marked, AsyncStreamCounter},
    socks5::Address,
};

use super::{Protocol, Stats};
use std::io::Write;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpProxy {
    pub address: Address<'static>,
    pub ssl: bool,
    pub auth_header: Option<String>,
}

#[async_trait]
impl Protocol for HttpProxy {
    type Stream = TlsStream<BufReader<AsyncStreamCounter<TcpStream>>>;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::Stream> {
        let mut upstream = AsyncStreamCounter::new(
            connect_tcp_marked(&self.address, fwmark)
                .await
                .context("Connecting to HTTP Proxy")?,
            stats.rx.clone(),
            stats.tx.clone(),
        );

        // Establish HTTP tunnel
        let mut request = Vec::new();
        let host = req.dst.get_host();
        let port = req.dst.get_port();
        write!(request, "CONNECT {host}:{port}\r\n")?;
        write!(request, "Host: {host}:{port}\r\n")?;
        if let Some(auth_header) = &self.auth_header {
            write!(request, "Proxy-Authorization: {}\r\n", auth_header)?;
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

        // Establish TLS if required
        let upstream = if self.ssl {
            TlsStream::connect_tls(host.as_ref(), upstream).await
        } else {
            TlsStream::connect_plain(upstream).await
        }
        .context("Connecting to TLS")?;

        Ok(upstream)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        protocol::test::{test_protocol_http, test_protocol_tcp},
        test::create_http_server,
    };

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
