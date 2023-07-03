use anyhow::{bail, Context};
use async_trait::async_trait;
use bytes::Bytes;
use hyper::header;
use std::sync::Arc;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::auth::AuthProvider;
use crate::addr::Address;
use crate::http::parse_request;
use crate::http::utils::WithHeaders;
use crate::http::writer::ResponseWriter;
use crate::protocol::ProxyRequest;

#[derive(Clone)]
pub struct HttpProxyAcceptor {
    pub auth_provider: Option<Arc<dyn AuthProvider>>,
}

pub struct HttpProxyAcceptedState {
    stream: BufReader<TcpStream>,
}

#[async_trait]
impl super::super::ProtocolAcceptor for HttpProxyAcceptor {
    type AcceptedState = HttpProxyAcceptedState;

    async fn accept(
        &self,
        stream: TcpStream,
    ) -> anyhow::Result<(Self::AcceptedState, ProxyRequest)> {
        let mut stream = BufReader::new(stream);

        let result = parse_request(&mut stream, |req| {
            // Only allow CONNECT
            match req.method {
                Some(m) if m.eq_ignore_ascii_case("CONNECT") => {}
                v => bail!("Expecting CONNECT but got {v:?}"),
            }

            // Parse the path as an address
            let addr: Address = req
                .path
                .context("Missing path")?
                .parse()
                .context("Parsing path as address")?;

            Ok((
                ProxyRequest::from(addr),
                req.get_header_text(header::PROXY_AUTHORIZATION)
                    .map(str::to_string),
            ))
        })
        .await;

        let (req, auth) = match result {
            Ok(v) => v,
            Err(err) => {
                ResponseWriter::write(400, "invalid parameters")
                    .to_async(&mut stream)
                    .await?;
                return Err(err);
            }
        };

        // Check auth
        if let Some(auth_provider) = &self.auth_provider {
            match auth_provider
                .check_auth(auth.as_ref().map(|v| v.as_ref()))
                .await
            {
                Ok(_) => {}
                Err(err) => {
                    ResponseWriter::write(403, "auth required")
                        .to_async(&mut stream)
                        .await?;
                    return Err(err);
                }
            }
        }

        Ok((Self::AcceptedState { stream }, req))
    }
}

#[async_trait]
impl super::super::ProtocolAcceptedState for HttpProxyAcceptedState {
    type ServerStream = BufReader<TcpStream>;

    async fn reply_success(
        mut self,
        initial_data: Option<Bytes>,
    ) -> anyhow::Result<Self::ServerStream> {
        ResponseWriter::write(200, "OK")
            .to_async(&mut self.stream)
            .await?;
        if let Some(data) = initial_data {
            self.stream
                .write_all(&data)
                .await
                .context("Writing initial data")?;
        }

        Ok(self.stream)
    }

    async fn reply_error(
        mut self,
        error: Option<impl AsRef<str> + Send + Sync>,
    ) -> anyhow::Result<()> {
        if let Some(error) = error {
            self.stream
                .write_all(
                    &ResponseWriter::write(500, "Internal error")
                        .finish_with_body("Content-Type: text/plain", error.as_ref().as_bytes()),
                )
                .await?;
        } else {
            ResponseWriter::write(500, "Internal error")
                .to_async(&mut self.stream)
                .await?;
        }

        Ok(())
    }
}
