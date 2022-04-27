use std::sync::Arc;

use crate::{
    io::{bind_udp, send_to_addr, UdpSocketExt},
    protocol::tcpman::dgram::{create_udp_sink, create_udp_stream},
    rt::{spawn, Task},
    utils::{new_vec_for_udp, race, VecExt},
};
use anyhow::Context;
use futures::{select, AsyncRead, AsyncReadExt, AsyncWrite, FutureExt, SinkExt, StreamExt};

use crate::{
    proxy::protocol::ProxyResult, rt::net::UdpSocket, socks5::Address,
    utils::write_bincode_lengthed_async,
};

async fn prepare_socket(
    v4: bool,
    initial_data: impl AsRef<[u8]> + Send,
    initial_dst: &Address<'static>,
) -> anyhow::Result<UdpSocket> {
    let socket = bind_udp(v4).await?;

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
    let (upstream_sink, upstream_stream) =
        match prepare_socket(v4, initial_data, &initial_dst).await {
            Ok(v) => {
                write_bincode_lengthed_async(
                    &mut stream,
                    &ProxyResult::Granted {
                        bound_address: None,
                        solved_addresses: None,
                    },
                )
                .await?;
                v.to_sink_stream().split()
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

    let (stream_r, stream_w) = stream.split();

    let udp_sink = create_udp_sink(stream_w);
    let udp_stream = create_udp_stream(stream_r);

    let upload_task = spawn(
        udp_stream
            .filter_map(|(buf, addr)| async move {
                match addr.resolve().await {
                    Ok(mut list) => match list.next() {
                        Some(v) => Some(anyhow::Result::Ok((buf, v))),
                        None => None,
                    },
                    Err(e) => {
                        log::error!("Unable to resolve {addr}: {e:?}");
                        Some(Err(e))
                    }
                }
            })
            .forward(upstream_sink),
    );

    let download_task = upstream_stream
        .map(|(data, addr)| anyhow::Result::Ok((data, Address::from(addr))))
        .forward(udp_sink);

    select! {
        _ = upload_task.fuse() => Ok(()),
        _ = download_task.fuse() => Ok(()),
    }
}

// #[cfg(test)]
// mod tests {
//     use std::time::Duration;

//     use crate::proxy::udp::write_packet_async;
//     use crate::rt::{block_on, TimeoutExt};

//     use crate::{
//         test::{duplex, echo_udp_server},
//         utils::read_bincode_lengthed_async,
//     };

//     use super::*;

//     async fn read_packet(r: &mut (impl AsyncRead + Unpin + Send + Sync)) -> Packet<Buf> {
//         Packet::read_async(r)
//             .timeout(Duration::from_secs(1))
//             .await
//             .unwrap()
//             .unwrap()
//     }

//     #[test]
//     fn serve_udp_proxy_conn_works() -> anyhow::Result<()> {
//         block_on(async move {
//             let (_udp_task, server_addr) = echo_udp_server().await;

//             let (mut near, far) = duplex(10).await;

//             let initial_data = b"hello, world";

//             let _task = spawn(serve_udp_proxy_conn(
//                 true,
//                 far,
//                 initial_data,
//                 server_addr.into(),
//             ));

//             // Must have received ProxyGranted
//             let result: ProxyResult = read_bincode_lengthed_async(&mut near).await?;
//             assert!(matches!(
//                 result,
//                 ProxyResult::Granted {
//                     bound_address: None,
//                     solved_addresses: None
//                 }
//             ));

//             // Must have received echo-ed data
//             let pkt = read_packet(&mut near).await;
//             assert_eq!(server_addr.port(), pkt.addr().unwrap().get_port());
//             assert_eq!(initial_data, pkt.payload());

//             let payload = b"second payload!";
//             write_packet_async(&mut near, Some(&server_addr.into()), payload)
//                 .await
//                 .unwrap();

//             let pkt = read_packet(&mut near).await;
//             assert_eq!(pkt.addr(), None);
//             assert_eq!(pkt.payload(), payload);

//             Ok(())
//         })
//     }
// }
