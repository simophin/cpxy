use std::borrow::Cow;

use anyhow::{bail, Context};
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{
    buf::RWBuffer,
    fetch::send_http,
    http::{parse_request, parse_response, AsyncHttpStream, HeaderValue, HttpCommon, HttpRequest},
};

pub async fn negotiate_websocket<'a>(
    url: &str,
    extra_headers: Vec<(Cow<'a, str>, HeaderValue<'a>)>,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync> {
    let u = crate::url::HttpUrl::try_from(url).context("Parsing server URL")?;
    let host = u.address.get_host();

    let mut req = HttpRequest {
        common: HttpCommon {
            headers: vec![
                (Cow::Borrowed("Connection"), "Upgrade".into()),
                (Cow::Borrowed("Upgrade"), "Websocket".into()),
                (Cow::Borrowed("Sec-WebSocket-Version"), "13".into()),
                (
                    Cow::Borrowed("Sec-WebSocket-Key"),
                    "dGhlIHNhbXBsZSBub25jZQ==".into(),
                ),
                (Cow::Borrowed("Host"), host.as_ref().into()),
            ],
        },
        method: Cow::Borrowed("GET"),
        path: u.path,
    };

    req.common.headers.extend(extra_headers);

    let http_stream = parse_response(
        send_http(u.is_https, &u.address, &req)
            .await
            .context("Sending initial request")?,
        RWBuffer::new(512, 65536),
    )
    .await
    .context("Parsing initial response")?;

    if http_stream.status_code != 101 {
        bail!("Expecting 101 response but got {}", http_stream.status_code);
    }

    Ok(http_stream)
}

pub struct WebSocketServeResult<T> {
    _sec_key: String,
    stream: AsyncHttpStream<HttpRequest<'static>, T>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> WebSocketServeResult<T> {
    pub async fn respond_success(
        mut self,
    ) -> anyhow::Result<AsyncHttpStream<HttpRequest<'static>, T>> {
        self.stream
            .write_all(
                b"HTTP/1.1 101 Switching Protocols\r\n\
                Upgrade: WebSocket\r\n\
                Connection: Upgrade\r\n\
                Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\
                \r\n",
            )
            .await?;
        Ok(self.stream)
    }

    pub async fn respond_fail_with_raw_response(
        mut self,
        http_response: &[u8],
    ) -> anyhow::Result<()> {
        self.stream.write_all(http_response).await?;
        Ok(())
    }

    pub fn request(&self) -> &HttpRequest {
        &self.stream
    }
}

pub async fn serve_websocket<T: AsyncRead + AsyncWrite + Unpin + Send + Sync>(
    stream: T,
) -> anyhow::Result<WebSocketServeResult<T>> {
    let mut req = parse_request(stream, RWBuffer::new(512, 65536))
        .await
        .map_err(|(e, _)| e)?;

    if !req.method.eq_ignore_ascii_case("get") {
        bail!("Expecting GET method but got {}", req.method);
    }

    let websocket_key = req.get_header_text("sec-websocket-key").unwrap_or_default();

    if req
        .get_header_text("connection")
        .unwrap_or_default()
        .eq_ignore_ascii_case("upgrade")
        && req
            .get_header_text("upgrade")
            .unwrap_or_default()
            .eq_ignore_ascii_case("websocket")
        && req
            .get_header_text("sec-websocket-version")
            .unwrap_or_default()
            .eq_ignore_ascii_case("13")
        && websocket_key.len() > 0
    {
        return Ok(WebSocketServeResult {
            _sec_key: websocket_key.into_owned(),
            stream: req,
        });
    }

    req.write_all(b"HTTP/1.1 404 Not found\r\n\r\n").await?;
    bail!("Invalid websocket parameters");
}
