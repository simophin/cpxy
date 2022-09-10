use anyhow::{bail, Context};
use futures::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{
    buf::RWBuffer,
    http::{
        parse_request, parse_response, AsyncHttpStream, HttpRequest, HttpRequestBuilder,
        WithHeaders,
    },
};

pub async fn negotiate_websocket(
    mut builder: HttpRequestBuilder,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync> {
    builder
        .put_header_text("Connection", "Upgrade")?
        .put_header_text("Upgrade", "Websocket")?
        .put_header_text("Sec-WebSocket-Version", "13")?
        .put_header_text("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")?;

    stream
        .write_all(&builder.finalise())
        .await
        .context("Sending request")?;

    let mut http_stream = parse_response(stream, RWBuffer::new_vec_uninitialised(512))
        .await
        .context("Parsing initial response")?;

    let status_code = http_stream.status_code;
    if status_code != 101 {
        bail!(
            "Expecting 101 response but got {}. Body: {:?}",
            status_code,
            http_stream
                .body()
                .await
                .ok()
                .and_then(|v| String::from_utf8(v).ok())
        );
    }

    Ok(http_stream)
}

pub struct WebSocketServeResult<T> {
    _sec_key: String,
    stream: AsyncHttpStream<HttpRequest<'static>, T>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> WebSocketServeResult<T> {
    pub async fn respond_success<'a>(
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
    let mut req = parse_request(stream, RWBuffer::new_vec_uninitialised(512))
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
            _sec_key: websocket_key.to_string(),
            stream: req,
        });
    }

    req.write_all(b"HTTP/1.1 404 Not found\r\n\r\n").await?;
    bail!("Invalid websocket parameters");
}
