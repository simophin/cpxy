use std::net::{IpAddr, Ipv4Addr};

use async_net::TcpStream;
use bytes::Buf;
use smol_timeout::TimeoutExt;

use super::*;
use smol::block_on;

async fn send_socks4_request(
    socks: &mut (impl AsyncRead + AsyncWrite + Unpin + Send + Sync),
    target: &Address<'_>,
) -> anyhow::Result<Address<'static>> {
    // Send request
    socks.write_all(&[0x4, 0x1]).await?;

    let port = target.get_port().to_be_bytes();
    socks.write_all(&port).await?;

    let hostname = match target {
        Address::IP(SocketAddr::V4(addr)) => {
            socks.write_all(addr.ip().octets().as_ref()).await?;
            None
        }
        Address::Name { host, .. } => {
            socks.write_all(&[0, 0, 0, 1]).await?;
            Some(host.as_ref())
        }
        _ => bail!("Unsupported IP protocol"),
    };

    // No user ID
    socks.write_all(&[0]).await?;

    if let Some(n) = hostname {
        socks.write_all(n.as_bytes()).await?;
        socks.write_all(&[0]).await?;
    }

    // Expect response
    let res = read_exact(socks, 8).await?;
    let mut res = res.as_slice();
    assert_eq!(res.get_u8(), 0);
    match res.get_u8() {
        0x5A => {}
        v => bail!("Error code {v}"),
    };
    let port = res.get_u16();
    let ip = Ipv4Addr::new(res[0], res[1], res[2], res[3]);

    Ok(Address::IP(SocketAddr::new(IpAddr::V4(ip), port)))
}

#[test]
fn test_socks4_tcp_proxy() {
    block_on(async move {
        let (_server, server_addr) = run_test_server().await;
        let (_client, client_addr) = run_test_client(server_addr).await;
        let (_echo_server, echo_server_addr) = echo_tcp_server().await;

        let mut socks4_client = TcpStream::connect(client_addr).await.unwrap();

        send_socks4_request(&mut socks4_client, &Address::IP(echo_server_addr))
            .timeout(TIMEOUT)
            .await
            .unwrap()
            .unwrap();

        let msg = b"hello, world";
        socks4_client.write_all(msg).await.unwrap();

        assert_eq!(
            read_exact(&mut socks4_client, msg.len())
                .timeout(TIMEOUT)
                .await
                .unwrap()
                .unwrap()
                .as_slice(),
            msg.as_ref()
        );
    })
}
