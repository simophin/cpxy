use crate::io::send_to_addr;
use crate::rt::TimeoutExt;

use crate::{
    buf::Buf,
    io::bind_udp,
    socks5::{UdpPacket as Socks5UdpPacket, UdpRepr as Socks5UdpRepr},
};

use super::*;

#[test]
fn test_udp() {
    let _ = env_logger::try_init();
    block_on(async move {
        let (_server, server_addr) = run_test_server().await;
        let (_client, client_addr) = run_test_client(server_addr).await;
        let (_echo_server, echo_server_addr) = echo_udp_server().await;

        let mut socks5_client = TcpStream::connect(client_addr).await.unwrap();

        let relay_addr = send_socks5_request(&mut socks5_client, &echo_server_addr.into(), true)
            .timeout(TIMEOUT)
            .await
            .unwrap()
            .unwrap();

        let socket = bind_udp(true).await.unwrap();

        let payload = b"hello, world";

        let pkt = Socks5UdpRepr {
            addr: echo_server_addr.into(),
            payload,
            frag_no: 0,
        }
        .to_packet()
        .unwrap();
        send_to_addr(&socket, pkt.inner().as_ref(), &relay_addr)
            .await
            .unwrap();

        let mut buf = Buf::new_for_udp();
        let (received, _) = socket
            .recv_from(&mut buf)
            .timeout(TIMEOUT)
            .await
            .unwrap()
            .unwrap();
        buf.set_len(received);
        let pkt = Socks5UdpPacket::new_checked(buf).unwrap();
        assert_eq!(pkt.payload(), payload.as_ref());
    });
}
