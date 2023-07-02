use anyhow::{bail, Context};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use std::fmt::Write;
use std::sync::Arc;
use tokio::io::{AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::auth::AuthProvider;
use crate::addr::Address;
use crate::http::parse_request;
use crate::http::utils::WithHeaders;
use crate::protocol::ProxyRequest;

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
                write_response(&mut stream, 400, "invalid parameters", Option::<&str>::None)
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
                    write_response(&mut stream, 403, "auth required", Option::<&str>::None).await?;
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
        write_response(&mut self.stream, 200, "OK", Option::<&str>::None).await?;
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
        write_response(&mut self.stream, 500, "Internal error", error).await
    }
}

async fn write_response(
    stream: &mut (impl AsyncWrite + Unpin),
    code: u16,
    reason: impl AsRef<str>,
    body: Option<impl AsRef<str>>,
) -> anyhow::Result<()> {
    let mut resp = BytesMut::new();

    write!(resp, "HTTP/1.1 {code} {}\r\n", reason.as_ref())?;

    if let Some(body) = body {
        write!(resp, "Content-Type: text/plain\r\n")?;
        write!(
            resp,
            "Content-Length: {}\r\n\r\n",
            body.as_ref().as_bytes().len()
        )?;
        resp.extend_from_slice(body.as_ref().as_bytes());
    } else {
        write!(resp, "\r\n")?;
    }

    stream.write_all(&resp).await.context("Writing response")?;
    Ok(())
}
