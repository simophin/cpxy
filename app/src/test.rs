use crate::cipher::client::connect;
use crate::cipher::server::listen;
use crate::cipher::strategy::EncryptionStrategy;
use crate::client::{run_client, ClientStatistics};
use crate::config::{ClientConfig, UpstreamConfig};
use crate::io::{TcpListener, TcpStream, UdpSocket};
use crate::proxy::protocol::{ProxyRequest, ProxyResult};
use crate::server::run_server;
use crate::socks5::{Address, UdpPacket};
use crate::utils::{
    read_bincode_lengthed_async, write_bincode_lengthed, write_bincode_lengthed_async, RWBuffer,
};
use futures_lite::future::race;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use futures_util::join;
use maplit::hashmap;
use rand::Rng;
use smol::channel::unbounded;
use smol::{spawn, Timer};
use smol_timeout::TimeoutExt;
use std::borrow::Cow;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

pub async fn duplex(
    _: usize,
) -> (
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) {
    let listener = TcpListener::bind(&"127.0.0.1:0".parse().unwrap())
        .await
        .expect("To listen");
    let addr = Address::IP(listener.local_addr().expect("To have local addr"));

    let (client, server) = join!(TcpStream::connect(&addr), listener.accept());
    let client = client.expect("To connect");
    let (server, _) = server.expect("To accept");

    (client, server)
}

#[test]
fn test_client_server_tcp() {
    let _ = env_logger::builder().is_test(true).try_init();
    smol::block_on(async move {
        let (client, server) = duplex(512).await;

        let client_send_enc = EncryptionStrategy::FirstN(NonZeroUsize::try_from(50).unwrap());
        let client_receive_enc = EncryptionStrategy::Never;

        let server_task = spawn(async move {
            let mut server = listen(server).await.expect("To create cipher channel");
            let req: ProxyRequest = read_bincode_lengthed_async(&mut server)
                .await
                .expect("To receive proxy request");

            assert!(matches!(req, ProxyRequest::TCP {dst} if dst == Address::default()));

            write_bincode_lengthed_async(
                &mut server,
                &ProxyResult::Granted {
                    bound_address: "1.2.3.4:8080".parse().ok(),
                    solved_addresses: None,
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

        let proxy_request = ProxyRequest::TCP {
            dst: Default::default(),
        };

        let mut req_buf = Vec::new();
        write_bincode_lengthed(&mut req_buf, &proxy_request).unwrap();

        let (client_r, client_w) = split(client);
        let mut client = connect(
            client_r,
            client_w,
            "localhost",
            client_send_enc,
            client_receive_enc,
            req_buf,
        )
        .await
        .unwrap();
        let result: ProxyResult = read_bincode_lengthed_async(&mut client).await.unwrap();

        assert!(
            matches!(result, ProxyResult::Granted {bound_address: Some(addr), ..} if addr.to_string() == "1.2.3.4:8080")
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
            })
            .detach();
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
        server_task.await
    });
}

#[test]
fn test_client_server_http() {}

async fn read_exact_n<T: AsyncRead + Unpin + Send + Sync, const N: usize>(
    r: &mut T,
) -> anyhow::Result<[u8; N]> {
    let mut buf = [0u8; N];
    r.read_exact(buf.as_mut_slice()).await?;
    Ok(buf)
}

#[test]
fn test_client_server_udp() {
    let _ = env_logger::builder().is_test(true).try_init();
    smol::block_on(async move {
        let udp_upstream = UdpSocket::bind(true).await.unwrap();
        let udp_upstream_addr = udp_upstream.local_addr().unwrap();
        log::info!("Upstream server listened at {udp_upstream_addr}");

        // Run the upstream UDP stream
        let _udp_task = spawn(async move {
            let mut buf = vec![0u8; 65536];
            loop {
                let (n, addr) = udp_upstream.recv_from(buf.as_mut_slice()).await.unwrap();
                buf.resize(n, 0);
                buf.extend_from_slice(b"+echo");
                udp_upstream.send_to(buf.as_slice(), addr).await.unwrap();
            }
        });

        let server = TcpListener::bind(&"127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let server_addr = server.local_addr().unwrap();

        // Run the proxy
        let (config_tx, config_rx) = unbounded();
        let _proxy_task = spawn(async move {
            race(run_client(config_rx), run_server(server))
                .await
                .unwrap();
        });

        config_tx
            .send((
                Arc::new(ClientConfig {
                    upstreams: hashmap! {
                        "test".to_string() => UpstreamConfig {
                            address: Address::IP(server_addr),
                            tls: false,
                            accept: Default::default(),
                            reject: Default::default(),
                            enabled: true,
                            priority: 0,
                        }
                    },
                    direct_accept: Default::default(),
                    direct_reject: Default::default(),
                    socks5_address: "127.0.0.1:5001".parse().unwrap(),
                    socks5_udp_host: "0.0.0.0".parse().unwrap(),
                }),
                Arc::new(ClientStatistics {
                    upstreams: hashmap! {
                        "test".to_string() => Default::default(),
                    },
                }),
            ))
            .await
            .unwrap();

        Timer::after(Duration::from_millis(100)).await;

        // Try to request a UDP proxy
        let mut socks5_client = TcpStream::connect(&"127.0.0.1:5001".parse().unwrap())
            .await
            .unwrap();

        // Greeting
        socks5_client.write_all(&[0x5, 1, 0x00]).await.unwrap();

        // Confirm auth
        assert_eq!(
            read_exact_n::<_, 2>(&mut socks5_client).await.unwrap(),
            [0x5, 0]
        );

        // Send proxy request
        socks5_client.write_all(&[0x5, 0x3, 0]).await.unwrap();
        Address::IP(udp_upstream_addr.clone())
            .write(&mut socks5_client)
            .await
            .unwrap();

        // Wait for proxy response
        assert_eq!(
            read_exact_n::<_, 3>(&mut socks5_client).await.unwrap(),
            [0x5, 0, 0]
        );
        let mut buf = RWBuffer::default();
        let addr = loop {
            match socks5_client.read(buf.write_buf()).await.unwrap() {
                0 => panic!("Unexpected EOF"),
                v => buf.advance_write(v),
            };

            match Address::parse(buf.read_buf()).unwrap() {
                None => continue,
                Some((offset, v)) => {
                    let v = v.to_owned();
                    buf.advance_read(offset);
                    break v;
                }
            }
        };
        log::info!("Address = {addr}");
        assert_eq!(buf.remaining_read(), 0);

        // Write to UDP address
        let udp_client = UdpSocket::bind(true).await.unwrap();
        let mut buf = Vec::<u8>::new();

        // Send first package
        buf.clear();
        UdpPacket {
            frag_no: 0,
            data: Cow::Borrowed(b"hello, world1"),
            addr: Address::IP(udp_upstream_addr.clone()),
        }
        .write_udp_sync(&mut buf)
        .unwrap();
        udp_client
            .send_to(buf.as_slice(), addr.to_string())
            .timeout(Duration::from_secs(3600))
            .await
            .unwrap()
            .unwrap();

        // Receive first package
        buf.resize(65536, 0);
        let (n, _) = udp_client
            .recv_from(buf.as_mut_slice())
            .timeout(Duration::from_secs(3600))
            .await
            .unwrap()
            .unwrap();
        buf.resize(n, 0);
        let received_pkt = UdpPacket::parse_udp(buf.as_slice()).unwrap();

        assert_eq!(received_pkt.data.as_ref(), b"hello, world1+echo");
    });
}
