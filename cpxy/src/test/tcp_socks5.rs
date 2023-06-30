use super::*;
use tokio::net::TcpStream;
use tokio::time::timeout;

#[tokio::test]
async fn test_tcp_socks5_proxy() {
    let _ = env_logger::try_init();
    let (_server, server_addr) = run_test_server().await;
    let (_client, client_addr) = run_test_client(server_addr).await;
    let (_echo_server, echo_server_addr) = echo_tcp_server().await;

    let mut socks5_client = TcpStream::connect(client_addr).await.unwrap();

    timeout(
        TIMEOUT,
        send_socks5_request(&mut socks5_client, &Address::IP(echo_server_addr), false),
    )
    .await
    .unwrap()
    .unwrap();

    let msg = b"hello, world";
    socks5_client.write_all(msg).await.unwrap();

    assert_eq!(
        timeout(TIMEOUT, read_exact(&mut socks5_client, msg.len()))
            .await
            .unwrap()
            .unwrap()
            .as_slice(),
        msg.as_ref()
    );
}
