use crate::{
    buf::RWBuffer,
    http::{parse_response, HttpRequestBuilder},
    test::echo_tcp_server,
};

use super::Protocol;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(1);

pub async fn test_protocol_tcp(p: &(impl Protocol + Send + Sync)) {
    let (_echo_task, echo_addr) = echo_tcp_server().await;

    let mut stream = timeout(
        TIMEOUT,
        p.new_stream(&echo_addr.into(), None, &Default::default(), None),
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
    let mut b = HttpRequestBuilder::new("GET", "/").unwrap();
    b.put_header_text("Host", "www.google.com").unwrap();
    let initial_data = b.finalise();

    let stream = timeout(
        TIMEOUT,
        p.new_stream(
            &"www.google.com:80".parse().unwrap(),
            Some(&initial_data),
            &Default::default(),
            None,
        ),
    )
    .await
    .expect("No timeout")
    .expect("To create new stream");

    let http_stream = parse_response(stream, RWBuffer::new_vec_uninitialised(4096))
        .await
        .expect("To parse response");

    assert_eq!(http_stream.status_code, 200);
}
