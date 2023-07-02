use crate::http::parse_request;
use crate::http::utils::WithHeaders;
use anyhow::Context;
use bytes::BytesMut;
use std::fmt::Write;
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
            req.check_header_value("Connection", "Upgrade")?;
            req.check_header_value("Upgrade", "websocket")?;
            req.check_header_value("Sec-WebSocket-Protocol", "mqtt")?;
            req.check_header_value("Sec-WebSocket-Version", "13")?;

            let key = req
                .get_header_text("Sec-WebSocket-Key")
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
        extra_headers: Option<impl FnOnce(&mut BytesMut) -> ()>,
    ) -> anyhow::Result<S>
    where
        S: AsyncWrite + Unpin,
    {
        let Self { mut stream, key } = self;
        let mut res = BytesMut::new();
        if is_success {
            write!(res, "HTTP/1.1 101 OK\r\n")?;
            write!(res, "Sec-WebSocket-Accept: {}\r\n", super::hash_ws_key(key))?;
            write!(res, "Sec-WebSocket-Protocol: mqtt\r\n")?;
        } else {
            write!(res, "HTTP/1.1 500 Internal Server Error\r\n")?;
        }

        if let Some(extra) = extra_headers {
            extra(&mut res);
        }

        if let Some(body) = &body {
            write!(
                res,
                "Content-Length: {}\r\n",
                body.as_ref().as_bytes().len()
            )?;
            write!(res, "Content-Type: text/plain\r\n")?;
        }

        write!(res, "\r\n")?;

        if let Some(body) = body {
            res.extend_from_slice(body.as_ref().as_bytes());
        }

        stream
            .write_all(&res)
            .await
            .context("Unable to write ws response")?;

        Ok(stream)
    }
}
