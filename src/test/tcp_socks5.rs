use crate::rt::TimeoutExt;

use super::*;

#[test]
fn test_tcp_socks5_proxy() {
    let _ = env_logger::try_init();
    block_on(async move {
        let (_server, server_addr) = run_test_server().await;
        let (_client, client_addr) = run_test_client(server_addr).await;
        let (_echo_server, echo_server_addr) = echo_tcp_server().await;

        let mut socks5_client = TcpStream::connect(client_addr).await.unwrap();

        send_socks5_request(&mut socks5_client, &Address::IP(echo_server_addr), false)
            .timeout(TIMEOUT)
            .await
            .unwrap()
            .unwrap();

        let msg = b"hello, world";
        socks5_client.write_all(msg).await.unwrap();

        assert_eq!(
            read_exact(&mut socks5_client, msg.len())
                .timeout(TIMEOUT)
                .await
                .unwrap()
                .unwrap()
                .as_slice(),
            msg.as_ref()
        );
    });
}
