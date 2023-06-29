use crate::http::parse_response;
use anyhow::Context;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::time::timeout;

use super::*;

#[tokio::test]
async fn test_http_proxy() {
    let _ = env_logger::try_init();
    let (_server, server_addr) = run_test_server().await;
    let (_client, client_addr) = run_test_client(server_addr).await;

    let mut res = timeout(
        TIMEOUT,
        fetch_http_with_proxy(
            "http://www.google.com",
            "GET",
            std::iter::empty(),
            &Address::IP(client_addr),
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(res.status_code, 200);
    assert!(!res.body().await.unwrap().is_empty());

    let mut res = timeout(
        TIMEOUT,
        fetch_http_with_proxy(
            "https://www.google.com",
            "GET",
            std::iter::empty(),
            &Address::IP(client_addr),
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(res.status_code, 200);
    assert!(!res.body().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_http_tunnel() {
    let _ = env_logger::try_init();
    let (_server, server_addr, password) = run_test_server().await;
    let (_client, client_addr) = run_test_client(server_addr, password).await;
    let (_echo_server, echo_server_addr) = echo_tcp_server().await;

    let mut proxy_client = TcpStream::connect(client_addr).await.unwrap();

    proxy_client
        .write_all(format!("CONNECT {echo_server_addr} HTTP/1.1\r\n\r\n").as_bytes())
        .await
        .unwrap();

    let mut proxy_client = BufReader::new(proxy_client);

    let status_code = timeout(
        TIMEOUT,
        parse_response(&mut proxy_client, |res| res.code.context("status code")),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(status_code, 200);

    let msg = b"hello, world, http tunnel";
    proxy_client.write_all(msg).await.unwrap();

    assert_eq!(
        timeout(TIMEOUT, read_exact(&mut http_stream, msg.len()))
            .await
            .unwrap()
            .unwrap()
            .as_slice(),
        msg.as_ref()
    );
}
