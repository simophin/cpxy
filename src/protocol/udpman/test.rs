use std::{borrow::Cow, time::Duration};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};

use super::*;
use crate::{
    io::bind_udp,
    protocol::Protocol,
    proxy::protocol::ProxyRequest,
    rt::{block_on, spawn, TimeoutExt, Timer},
    test::{echo_udp_server, set_ip_local},
};

#[test]
fn serve_works() {
    let _ = env_logger::try_init();
    block_on(async move {
        let (_echo_task, echo_server_addr) = echo_udp_server().await;

        let server_socket = bind_udp(true).await.unwrap();
        let mut udp_man_server_addr = server_socket.local_addr().unwrap();
        set_ip_local(&mut udp_man_server_addr);
        let _udp_man_task = spawn(server::serve_socket(server_socket));

        let _ = Timer::after(Duration::from_millis(500)).await;

        let protocol = UdpMan {
            addr: udp_man_server_addr.into(),
        };

        // Initial response
        let data = b"hello, world";
        let (mut dgram_sink, mut dgram_stream) = protocol
            .new_dgram_conn(
                &ProxyRequest::UDP {
                    initial_dst: echo_server_addr.into(),
                    initial_data: Cow::Borrowed(data),
                },
                &Default::default(),
                None,
            )
            .await
            .unwrap();
        let timeout = Duration::from_secs(100);
        let received = dgram_stream
            .next()
            .timeout(timeout)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        assert_eq!(data, received.0.as_ref());
        assert_eq!(echo_server_addr.port(), received.1.get_port());

        // Subsequent responses
        let data = b"second, data";
        dgram_sink
            .send((Bytes::from_static(data), echo_server_addr.into()))
            .await
            .unwrap();
        let received = dgram_stream
            .next()
            .timeout(timeout)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        assert_eq!(data, received.0.as_ref());
        assert_eq!(echo_server_addr.port(), received.1.get_port());
    });
}
