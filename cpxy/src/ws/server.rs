use crate::http::parse_request;
use crate::http::utils::WithHeaders;
use crate::http::writer::{HeaderWriter, ResponseWriter};
use anyhow::Context;
use hyper::header;
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt};

pub struct Acceptor<S> {
    stream: S,
    key: String,
}

impl<S> Acceptor<S> {
    pub async fn accept<T>(
        mut stream: S,
        extract_request: impl FnOnce(&httparse::Request<'_, '_>) -> anyhow::Result<T>,
    ) -> anyhow::Result<(Self, T)>
    where
        S: AsyncBufRead + Unpin,
    {
        let (key, result) = parse_request(&mut stream, move |req| {
            req.check_header_value(header::CONNECTION, "Upgrade")?;
            req.check_header_value(header::UPGRADE, "websocket")?;
            req.check_header_value(header::SEC_WEBSOCKET_PROTOCOL, "mqtt")?;
            req.check_header_value(header::SEC_WEBSOCKET_VERSION, "13")?;

            let key = req
                .get_header_text(header::SEC_WEBSOCKET_KEY)
                .context("Expecting key")?;

            let result = extract_request(req)?;

            Ok((key.to_string(), result))
        })
        .await?;

        Ok((Self { stream, key }, result))
    }

    pub async fn respond(
        self,
        is_success: bool,
        body: Option<impl AsRef<str> + Send + Sync>,
        extra_headers: Option<impl FnOnce(&mut HeaderWriter) -> ()>,
    ) -> anyhow::Result<S>
    where
        S: AsyncWrite + Unpin,
    {
        let Self { mut stream, key } = self;
        let mut res = if is_success {
            let mut w = ResponseWriter::write(101, "Switching Protocol");
            w.write_header(header::SEC_WEBSOCKET_ACCEPT, super::hash_ws_key(key));
            w.write_header(header::SEC_WEBSOCKET_PROTOCOL, "mqtt");
            w.write_header(header::SEC_WEBSOCKET_VERSION, "13");
            w
        } else {
            ResponseWriter::write(500, "Internal server error")
        };

        if let Some(extra) = extra_headers {
            extra(&mut res);
        }

        let buf = if let Some(body) = body {
            res.finish_with_body("text/plain", body.as_ref().as_bytes())
        } else {
            res.finish()
        };

        stream
            .write_all(&buf)
            .await
            .context("Unable to write ws response")?;

        Ok(stream)
    }
}
