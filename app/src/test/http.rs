use crate::{
    fetch::send_http,
    http::{parse_response, HttpRequest},
};

use super::*;

#[test]
fn test_http_proxy() {
    let _ = env_logger::try_init();
    block_on(async move {
        let (_server, server_addr) = run_test_server().await;
        let (_client, client_addr) = run_test_client(server_addr).await;

        let mut res = fetch_http_with_proxy(
            "http://www.google.com",
            "GET",
            std::iter::empty(),
            &Address::IP(client_addr),
            None,
        )
        .await
        .unwrap();

        assert_eq!(res.status_code, 200);
        assert!(!res.body().await.unwrap().is_empty());

        let mut res = fetch_http_with_proxy(
            "https://www.google.com",
            "GET",
            std::iter::empty(),
            &Address::IP(client_addr),
            None,
        )
        .await
        .unwrap();

        assert_eq!(res.status_code, 200);
        assert!(!res.body().await.unwrap().is_empty());
    });
}

#[test]
fn test_http_tunnel() {
    block_on(async move {
        let (_server, server_addr) = run_test_server().await;
        let (_client, client_addr) = run_test_client(server_addr).await;
        let (_echo_server, echo_server_addr) = echo_tcp_server().await;

        let mut socks5_client = TcpStream::connect_raw(client_addr).await.unwrap();
        socks5_client
            .write_all(format!("CONNECT {echo_server_addr}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut http_stream = parse_response(socks5_client, Default::default())
            .await
            .unwrap();

        assert_eq!(http_stream.status_code, 200);

        let msg = b"hello, world, http tunnel";
        http_stream.write_all(msg).await.unwrap();

        assert_eq!(
            read_exact(&mut http_stream, msg.len())
                .await
                .unwrap()
                .as_slice(),
            msg.as_ref()
        );
    });
}
