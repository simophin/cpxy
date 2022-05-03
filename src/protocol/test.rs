use async_stream::stream;
use bytes::Bytes;

use crate::{
    buf::RWBuffer,
    http::{parse_response, HttpRequest},
    proxy::protocol::ProxyRequest,
    rt::TimeoutExt,
    test::{echo_tcp_server, echo_udp_server},
};

use super::Protocol;
use futures::{AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt};
use std::{borrow::Cow, io::Write, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(1);

pub async fn test_protocol_tcp(p: &impl Protocol) {
    let (_echo_task, echo_addr) = echo_tcp_server().await;

    let req = ProxyRequest::TCP {
        dst: echo_addr.into(),
    };
    assert!(p.supports(&req));

    let (mut stream, _) = p
        .new_stream_conn(&req, &Default::default(), None)
        .timeout(TIMEOUT)
        .await
        .expect("No timeout")
        .expect("To create new stream connection");

    let mut test_data = Box::pin(stream! {
        for i in 0..1000 {
            let mut buf = vec![0u8; 0];
            write!(&mut buf, "Test data {i}").unwrap();
            yield buf;
        }
    });

    let mut recv_buf = vec![];
    while let Some(data) = test_data.next().await {
        stream.write_all(&data).await.expect("To write test data");

        recv_buf.resize(data.len(), 0);
        stream
            .read_exact(&mut recv_buf)
            .timeout(TIMEOUT)
            .await
            .expect("No timeout")
            .expect("To receive test data");

        assert_eq!(data, recv_buf);
    }
}

pub async fn test_protocol_http(p: &impl Protocol) {
    let req = ProxyRequest::HTTP {
        dst: "www.google.com:80".parse().unwrap(),
        https: false,
        req: HttpRequest {
            headers: Default::default(),
            method: Cow::Borrowed("GET"),
            path: Cow::Borrowed("/"),
        },
    };

    assert!(p.supports(&req));

    let (stream, _) = p
        .new_stream_conn(&req, &Default::default(), None)
        .timeout(TIMEOUT)
        .await
        .expect("No timeout")
        .expect("To create new stream");

    let http_stream = parse_response(stream, RWBuffer::new_vec_uninitialised(4096))
        .await
        .expect("To parse response");

    assert_eq!(http_stream.status_code, 200);
}

pub async fn test_protocol_udp(p: &impl Protocol) {
    let (_echo_task, echo_addr) = echo_udp_server().await;

    let initial_data = b"hello, world";
    let req = ProxyRequest::UDP {
        initial_dst: echo_addr.into(),
        initial_data: Cow::Borrowed(initial_data),
    };
    assert!(p.supports(&req));

    let (mut sink, mut stream) = p
        .new_dgram_conn(&req, &Default::default(), None)
        .timeout(TIMEOUT)
        .await
        .expect("No timeout")
        .expect("To create new dgram conn");

    let initial_reply = stream
        .next()
        .timeout(TIMEOUT)
        .await
        .expect("No timeout")
        .expect("Receive initial response")
        .expect("Receive initial response");

    assert_eq!(initial_data, initial_reply.0.as_ref());
    assert_eq!(echo_addr.port(), initial_reply.1.get_port());

    let mut test_data = Box::pin(stream! {
        for i in 0..1000 {
            let mut buf = vec![0u8; 0];
            write!(&mut buf, "Test data {i}").unwrap();
            yield Bytes::from(buf);
        }
    });

    while let Some(data) = test_data.next().await {
        sink.send((data.clone(), echo_addr.into()))
            .await
            .expect("To send test data");

        let (received, from) = stream
            .next()
            .timeout(TIMEOUT)
            .await
            .expect("No timeout")
            .expect("To have receive something")
            .expect("Receive msg correctly");

        assert_eq!(received, data);
        assert_eq!(echo_addr.port(), from.get_port());
    }
}
