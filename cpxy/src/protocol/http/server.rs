use anyhow::{bail, Context};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::io::{AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::auth::{AuthProvider, BasicAuthSettings};
use crate::addr::Address;
use crate::http::parse_request;
use crate::http::utils::WithHeaders;
use crate::protocol::{ProtocolReply, ProxyRequest};

pub struct HttpProxyAcceptor {
    auth_provider: Option<Arc<dyn AuthProvider>>,
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
                req.get_header_text("Proxy-Authenticate")
                    .map(str::to_string),
            ))
        })
        .await;

        let (req, auth) = match result {
            Ok(v) => v,
            Err(err) => {
                write_response(&mut stream, 400, "invalid parameters").await?;
                return Err(err);
            }
        };

        // Check auth
        if let Some(auth_provider) = &self.auth_provider {
            let auth = auth.map(|v| v.as_ref());
            match auth_provider.check_auth(auth).await {
                Ok(_) => {}
                Err(err) => {
                    write_response(&mut stream, 403, "auth required").await?;
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

    async fn reply(mut self, reply: ProtocolReply) -> anyhow::Result<Self::ServerStream> {
        if reply {
            write_response(&mut self.stream, 200, "OK").await?;
        } else {
            write_response(&mut self.stream, 500, "Internal error").await?;
        }

        Ok(self.stream)
    }
}

async fn write_response(
    stream: &mut (impl AsyncWrite + Unpin),
    code: u16,
    reason: impl AsRef<str>,
) -> anyhow::Result<()> {
    stream
        .write_all(format!("HTTP/1.1 {code} {}\r\n\r\n", reason.as_ref()).as_bytes())
        .await
        .context("Writing response")?;
    Ok(())
}
