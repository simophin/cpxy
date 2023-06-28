use crate::{http::parse_response, test::echo_tcp_server};

use super::Protocol;
use crate::protocol::ProxyRequest;
use anyhow::Context;
use bytes::Bytes;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(1);

pub async fn test_protocol_tcp(p: &(impl Protocol + Send + Sync)) {
    let (_echo_task, echo_addr) = echo_tcp_server().await;

    let mut stream = timeout(
        TIMEOUT,
        p.new_stream(
            &ProxyRequest::plain_tcp(echo_addr.into()),
            &Default::default(),
            None,
        ),
    )
    .await
    .expect("No timeout")
    .expect("To create new stream connection");

    let test_data = (0..1000).map(|i| format!("Test data {}", i).into_bytes());

    let mut recv_buf = vec![];
    for data in test_data {
        stream.write_all(&data).await.expect("To write test data");

        recv_buf.resize(data.len(), 0);
        timeout(TIMEOUT, stream.read_exact(&mut recv_buf))
            .await
            .expect("No timeout")
            .expect("To receive test data");

        assert_eq!(data, recv_buf);
    }
}

pub async fn test_protocol_http(p: &(impl Protocol + Send + Sync)) {
    let initial_data = Bytes::from_static(b"GET / HTTP/1.1\r\nHost: www.google.com\r\n\r\n");

    let stream = timeout(
        TIMEOUT,
        p.new_stream(
            &ProxyRequest {
                dst: "www.google.com:80".parse().unwrap(),
                initial_data: Some(initial_data),
                tls: false,
            },
            &Default::default(),
            None,
        ),
    )
    .await
    .expect("No timeout")
    .expect("To create new stream");

    let mut stream = BufReader::new(stream);

    let status_code = parse_response(&mut stream, |res| res.code.context("status code"))
        .await
        .expect("to parse response");

    assert_eq!(status_code, 200);
}
