use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use futures_lite::{future::race, io::split, AsyncRead, AsyncWrite};
use smol::{spawn, Task};

use crate::{
    buf::Buf,
    io::UdpSocket,
    proxy::udp::{write_packet_async, Packet},
    socks5::Address,
};

pub async fn serve_udp_proxy_conn(
    v4: bool,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    initial_data: impl AsRef<[u8]> + Send,
    initial_dst: Address<'static>,
) -> anyhow::Result<()> {
    let socket = Arc::new(UdpSocket::bind(v4).await?);

    socket
        .send_to_addr(initial_data.as_ref(), &initial_dst)
        .await
        .context("Sending initial data")?;

    log::debug!(
        "Sending initial data(len={}) to {initial_dst}",
        initial_data.as_ref().len()
    );

    let close_on_receive = initial_dst.get_port() == 53;

    let (mut stream_r, mut stream_w) = split(stream);

    let task1: Task<anyhow::Result<()>> = {
        let socket = socket.clone();
        spawn(async move {
            let mut last_addr = initial_dst;
            loop {
                let pkt = Packet::read_async(&mut stream_r).await?;
                let dst = match (pkt.addr(), &last_addr) {
                    (Some(requested), _) => {
                        last_addr = requested.into_owned();
                        &last_addr
                    }
                    (None, last) => last,
                };

                log::debug!("Sending payload(len={}) to {dst}", pkt.payload().len());
                socket.send_to_addr(pkt.payload(), dst).await?;
            }
        })
    };

    let task2: Task<anyhow::Result<()>> = spawn(async move {
        let mut last_addr: Option<SocketAddr> = None;
        loop {
            let mut buf = Buf::new_for_udp();
            let (len, from) = socket.recv_from(&mut buf).await?;
            buf.set_len(len);
            let addr = match (from, last_addr.as_ref()) {
                (v, Some(last)) if last == &v => None,
                (v, _) => {
                    last_addr.replace(v);
                    Some(v)
                }
            };

            let written =
                write_packet_async(&mut stream_w, addr.map(|a| a.into()).as_ref(), &buf).await?;
            log::debug!(
                "Received {len} bytes from UDP://{from}, written {written} bytes back to TCP stream"
            );

            if close_on_receive {
                log::debug!("Closing because of DNS request");
                break Ok(());
            }
        }
    });

    race(task1, task2).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use smol::block_on;
    use smol_timeout::TimeoutExt;

    use crate::test::{duplex, echo_udp_server};

    use super::*;

    async fn read_packet(r: &mut (impl AsyncRead + Unpin + Send + Sync)) -> Packet<Buf> {
        Packet::read_async(r)
            .timeout(Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap()
    }

    #[test]
    fn serve_udp_proxy_conn_works() -> anyhow::Result<()> {
        block_on(async move {
            let (_udp_task, server_addr) = echo_udp_server().await;

            let (mut near, far) = duplex(10).await;

            let initial_data = b"hello, world";

            let _task = spawn(serve_udp_proxy_conn(
                true,
                far,
                initial_data,
                server_addr.into(),
            ));

            // Must have received echo-ed data
            let pkt = read_packet(&mut near).await;
            assert_eq!(server_addr.port(), pkt.addr().unwrap().get_port());
            assert_eq!(initial_data, pkt.payload());

            let payload = b"second payload!";
            write_packet_async(&mut near, Some(&server_addr.into()), payload)
                .await
                .unwrap();

            let pkt = read_packet(&mut near).await;
            assert_eq!(pkt.addr(), None);
            assert_eq!(pkt.payload(), payload);

            Ok(())
        })
    }
}
