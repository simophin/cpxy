use crate::cipher::client::connect;
use crate::cipher::server::listen;
use crate::proxy::handler::{
    receive_proxy_request, request_proxy, send_proxy_result, ProxyRequest, ProxyResult,
};
use crate::socks5::Address;
use crate::utils::RWBuffer;
use rand::Rng;
use std::time::Duration;
use tokio::io::{duplex, split, AsyncReadExt, AsyncWriteExt};
use tokio::spawn;
use tokio::time::timeout;

#[tokio::test]
async fn test_client_server() {
    env_logger::init();
    let (client, server) = duplex(512);
    let duration = Duration::from_secs(10);

    let server_task = spawn(async move {
        let mut server = listen(server).await.expect("To create cipher channel");
        let req = receive_proxy_request(&mut server)
            .await
            .expect("To receive proxy request");

        assert!(matches!(req, ProxyRequest::SocksTCP(addr) if addr == Address::default()));

        send_proxy_result(
            &mut server,
            ProxyResult::Granted {
                bound_address: "1.2.3.4:8080".parse().unwrap(),
            },
        )
        .await
        .expect("To send proxy result");

        let mut data = vec![0u8; 4096];
        loop {
            match server.read(data.as_mut_slice()).await {
                Ok(v) if v == 0 => {
                    log::info!("EOF");
                    break;
                }
                Err(e) => panic!("Error reading: {e}"),
                Ok(v) => {
                    log::info!("server: Received {v} bytes");
                    server
                        .write_all(&data.as_slice()[..v])
                        .await
                        .expect("To write");
                    server.flush().await.expect("To flush");
                    log::info!("server: Written {v} bytes");
                }
            }
        }
    });

    let (result, client) = request_proxy(
        &ProxyRequest::SocksTCP(Default::default()),
        move |buf| async move { connect(client, buf).await },
    )
    .await
    .expect("To request proxy");

    assert!(
        matches!(result, ProxyResult::Granted {bound_address} if bound_address.to_string() == "1.2.3.4:8080")
    );

    let (mut r, mut w) = split(client);

    let mut data_to_send = vec![0u8; 8192];
    rand::thread_rng().fill(data_to_send.as_mut_slice());

    {
        let data_to_send = data_to_send.clone();
        spawn(async move {
            w.write_all(data_to_send.as_slice())
                .await
                .expect("To write data");
            w.flush().await.expect("to flush");
            log::debug!("client: Sent {} data", data_to_send.len());
        });
    }

    let mut data_received = RWBuffer::with_capacity(data_to_send.len());
    while data_received.remaining_write() > 0 {
        match r.read(data_received.write_buf()).await.expect("To read") {
            0 => panic!("Zero read"),
            v => {
                data_received.advance_write(v);
                log::debug!(
                    "Client: Received {v} bytes. Total: {}, remaining_write: {}",
                    data_received.remaining_read(),
                    data_received.remaining_write(),
                );
            }
        }
    }

    assert_eq!(data_to_send.as_slice(), data_received.read_buf());
    drop(r);
    server_task.await.unwrap();
}
