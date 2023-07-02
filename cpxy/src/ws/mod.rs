use base64::Engine;
use sha1_smol::Sha1;

pub mod client;
pub mod server;

use base64::engine::general_purpose::STANDARD as B64;

fn hash_ws_key(key: impl AsRef<str>) -> String {
    let mut m = Sha1::new();
    m.update(key.as_ref().as_bytes());
    m.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    B64.encode(m.digest().bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::utils::WithHeaders;
    use crate::ws::server::Acceptor;
    use anyhow::Context;
    use bytes::BytesMut;
    use std::fmt::Write;
    use tokio::io::{duplex, BufReader};
    use tokio::spawn;

    #[tokio::test]
    async fn test_ws() {
        let (client, server) = duplex(4096);

        let received = spawn(async move {
            let (acceptor, _) = Acceptor::accept(BufReader::new(server), |req| {
                req.check_header_value("Request-Data", "data")
            })
            .await?;

            acceptor
                .respond(
                    true,
                    Option::<&str>::None,
                    Some(|buf: &mut BytesMut| {
                        write!(buf, "Response-Data: hello\r\n").unwrap();
                    }),
                )
                .await
        });

        let mut client = BufReader::new(client);
        let response_data = client::connect(
            &mut client,
            "/path",
            "host",
            Some(|buf: &mut BytesMut| {
                write!(buf, "Request-Data: data\r\n").unwrap();
            }),
            |res| {
                res.get_header_text("Response-Data")
                    .context("Response data")
                    .map(|v| v.to_string())
            },
        )
        .await
        .expect("to connect");

        assert_eq!(response_data, "hello");
        let _ = received
            .await
            .expect("Joining error")
            .expect("To serve successfully");
    }
}
