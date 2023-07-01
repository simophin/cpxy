use crate::http::parse_request;
use crate::http::utils::WithHeaders;
use anyhow::{bail, Context};
use bytes::BytesMut;
use tokio::io::{AsyncBufRead, AsyncWrite};

pub async fn accept_ws<T>(
    stream: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    extract_request: impl FnOnce(&httparse::Request<'_, '_>, &mut BytesMut) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    parse_request(stream, move |req| {
        req.check_header_value("Connection", "Upgrade")?;
        req.check_header_value("Upgrade", "websocket")?;
        req.check_header_value("Sec-WebSocket-Protocol", "mqtt")?;
        req.check_header_value("Sec-WebSocket-Version", "13")?;

        let key = req
            .get_header_text("Sec-WebSocket-Key")
            .context("Expecting key")?;

        todo!()
    })
    .await?
}
