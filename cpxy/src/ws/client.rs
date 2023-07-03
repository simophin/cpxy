use super::{hash_ws_key, B64};
use crate::http::parse_response;
use crate::http::utils::WithHeaders;
use crate::http::writer::{HeaderWriter, RequestWriter};
use anyhow::{bail, Context};
use base64::Engine;
use hyper::{header, Method};
use once_cell::sync::Lazy;
use rand::Rng;
use tokio::io::{AsyncBufRead, AsyncWrite};

pub async fn connect<T>(
    stream: &mut (impl AsyncBufRead + AsyncWrite + Unpin),
    path: impl AsRef<str>,
    host: impl AsRef<str>,
    extra_request_header: Option<impl FnOnce(&mut HeaderWriter)>,
    extract_response: impl FnOnce(&httparse::Response) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut req = RequestWriter::write(Method::GET, path.as_ref());
    let key = B64.encode(rand::random::<[u8; 16]>());

    req.write_header(header::HOST, host.as_ref())
        .write_header(header::CONNECTION, "Upgrade")
        .write_header(header::UPGRADE, "websocket")
        .write_header(header::SEC_WEBSOCKET_PROTOCOL, "mqtt")
        .write_header(header::SEC_WEBSOCKET_VERSION, "13")
        .write_header(header::REFERER, host.as_ref())
        .write_header(header::USER_AGENT, pick_random_user_agent())
        .write_header(header::SEC_WEBSOCKET_KEY, &key);

    if let Some(cb) = extra_request_header {
        cb(&mut req);
    }

    req.to_async(stream)
        .await
        .context("Writing websocket request")?;

    // Wait for response
    parse_response(stream, move |res| {
        if res.code != Some(101) {
            bail!("Expecting protocol switch status but got: {:?}", res.code);
        }

        res.check_header_value(header::SEC_WEBSOCKET_PROTOCOL, "mqtt")?;

        let accept_key = res
            .get_header_text(header::SEC_WEBSOCKET_ACCEPT)
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
