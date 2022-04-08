use std::borrow::Cow;

use smol_timeout::TimeoutExt;

use crate::socks5::UdpPacket;

use super::*;

#[test]
fn test_udp() {
    let _ = env_logger::try_init();
    block_on(async move {
        // let (_server, server_addr) = run_test_server().await;
        // let (_client, client_addr) = run_test_client(server_addr).await;
        // let (_echo_server, echo_server_addr) = echo_udp_server().await;
        // let echo_server_addr = Address::IP(echo_server_addr);

        // let mut socks5_client = TcpStream::connect_raw(client_addr).await.unwrap();

        // let relay_addr = send_socks5_request(&mut socks5_client, &echo_server_addr, true)
        //     .timeout(TIMEOUT)
        //     .await
        //     .unwrap()
        //     .unwrap();

        // let socket = UdpSocket::bind(true).await.unwrap();
        // let mut buf = Vec::new();

        // let msg = b"hello, world";

        // UdpPacket {
        //     addr: echo_server_addr.clone(),
        //     data: Cow::Borrowed(msg),
        //     frag_no: 0,
        // }
        // .write_udp_sync(&mut buf)
        // .unwrap();
        // socket.send_to_addr(&buf, &relay_addr).await.unwrap();

        // buf.resize(65536, 0);
        // let (received, _) = socket
        //     .recv_from(buf.as_mut_slice())
        //     .timeout(TIMEOUT)
        //     .await
        //     .unwrap()
        //     .unwrap();
        // let pkt = UdpPacket::parse_udp(&buf[..received]).unwrap();
        // assert_eq!(pkt.data.as_ref(), msg.as_ref());
    });
}
