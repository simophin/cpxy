use std::{net::SocketAddr, sync::Arc};

use crate::{
    io::{bind_udp, send_to_addr},
    rt::{spawn, Task},
};
use anyhow::Context;
use futures_lite::{future::race, io::split, AsyncRead, AsyncWrite, StreamExt};

use crate::{
    buf::Buf,
    proxy::{
        protocol::ProxyResult,
        udp::{Packet, PacketWriter},
    },
    rt::net::UdpSocket,
    socks5::Address,
    utils::write_bincode_lengthed_async,
};

async fn prepare_socket(
    v4: bool,
    initial_data: impl AsRef<[u8]> + Send,
    initial_dst: &Address<'static>,
) -> anyhow::Result<Arc<UdpSocket>> {
    let socket = Arc::new(bind_udp(v4).await?);

    send_to_addr(&socket, initial_data.as_ref(), &initial_dst)
        .await
        .context("Sending initial data")?;

    log::debug!(
        "Sending initial data(len={}) to {initial_dst}",
        initial_data.as_ref().len()
    );
    Ok(socket)
}

pub async fn serve_udp_proxy_conn(
    v4: bool,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    initial_data: impl AsRef<[u8]> + Send,
    initial_dst: Address<'static>,
) -> anyhow::Result<()> {
    let socket = match prepare_socket(v4, initial_data, &initial_dst).await {
        Ok(v) => {
            write_bincode_lengthed_async(
                &mut stream,
                &ProxyResult::Granted {
                    bound_address: None,
                    solved_addresses: None,
                },
            )
            .await?;
            v
        }
        Err(e) => {
            write_bincode_lengthed_async(
                &mut stream,
                &ProxyResult::ErrGeneric { msg: e.to_string() },
            )
            .await?;
            return Err(e);
        }
    };

    let close_on_receive = initial_dst.get_port() == 53;

    let (mut stream_r, mut stream_w) = split(stream);

    let task1: Task<anyhow::Result<()>> = {
        let socket = socket.clone();
        spawn(async move {
            let mut packet_stream = Packet::new_packet_stream(stream_r, Some(initial_dst));

            while let Some((buf, addr)) = packet_stream.next().await {
                log::debug!("Sending payload(len={}) to {addr}", buf.as_ref().len());
                send_to_addr(&socket, buf.as_ref(), &addr).await?;
            }

            Ok(())
        })
    };

    let task2: Task<anyhow::Result<()>> = spawn(async move {
        let mut packet_writer = PacketWriter::new();
        loop {
            let mut buf = Buf::new_for_udp();
            let (len, from) = socket.recv_from(&mut buf).await?;
            buf.set_len(len);

            let written = packet_writer
                .write(&mut stream_w, &from.into(), &buf)
                .await?;
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

    use crate::proxy::udp::write_packet_async;
    use crate::rt::{block_on, TimeoutExt};

    use crate::{
        test::{duplex, echo_udp_server},
        utils::read_bincode_lengthed_async,
    };

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

            // Must have received ProxyGranted
            let result: ProxyResult = read_bincode_lengthed_async(&mut near).await?;
            assert!(matches!(
                result,
                ProxyResult::Granted {
                    bound_address: None,
                    solved_addresses: None
                }
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
