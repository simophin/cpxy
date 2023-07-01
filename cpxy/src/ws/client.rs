use std::fmt::Write;

use super::{hash_ws_key, B64};
use crate::http::parse_response;
use crate::http::utils::WithHeaders;
use anyhow::{bail, Context};
use base64::Engine;
use bytes::BytesMut;
use once_cell::sync::Lazy;
use rand::Rng;
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt};

pub async fn connect_ws<T>(
    stream: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    path: impl AsRef<str>,
    host: impl AsRef<str>,
    extra_request_header: Option<impl FnOnce(&mut BytesMut)>,
    extract_response: impl FnOnce(&httparse::Response) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut req = BytesMut::new();
    write!(req, "GET {} HTTP/1.1\r\n", path.as_ref())?;
    write!(req, "Host: {}\r\n", host.as_ref())?;
    write!(req, "Connection: Upgrade\r\n")?;
    write!(req, "Upgrade: websocket\r\n")?;
    write!(req, "Sec-WebSocket-Protocol: mqtt\r\n")?;
    write!(req, "Sec-WebSocket-Version: 13\r\n")?;
    write!(req, "Referer: http://{}\r\n", host.as_ref())?;
    write!(req, "User-Agent: {}\r\n", pick_random_user_agent())?;

    let key = B64.encode(rand::random::<[u8; 16]>());

    write!(req, "Sec-WebSocket-Key: {key}\r\n",)?;

    if let Some(cb) = extra_request_header {
        cb(&mut req);
    }
    write!(req, "\r\n")?;

    stream
        .write_all(&req)
        .await
        .context("Writing websocket request")?;

    // Wait for response
    parse_response(stream, move |res| {
        if res.code != Some(101) {
            bail!("Expecting protocol switch status but got: {:?}", res.code);
        }

        let accept_key = res
            .get_header_text("Sec-WebSocket-Accept")
            .context("expecting Sec-WebSocket-Accept")?;

        let expecting = hash_ws_key(key);
        if accept_key != expecting {
            bail!("WS expecting {expecting} but got {accept_key}")
        }

        extract_response(res)
    })
    .await
}

static AGENTS_CELL: Lazy<Vec<&str>> = Lazy::new(|| {
    let all_agents = include_str!("common_user_agents.txt");
    all_agents.split('\n').collect()
});

fn pick_random_user_agent() -> &'static str {
    let agents = AGENTS_CELL.as_slice();
    let index = rand::thread_rng().gen_range(0..agents.len());
    agents[index]
}
