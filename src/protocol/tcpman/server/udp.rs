use crate::{
    io::{bind_udp, send_to_addr, UdpSocketExt},
    protocol::tcpman::dgram::{create_udp_sink, create_udp_stream},
    rt::spawn,
};
use anyhow::Context;
use futures::{
    select, AsyncRead, AsyncReadExt, AsyncWrite, FutureExt, SinkExt, StreamExt, TryStreamExt,
};

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
    let udp_stream = create_udp_stream(stream_r, Some(initial_dst));

    let upload_task = spawn(
        udp_stream
            .filter_map(|item| async move {
                let (buf, addr) = match item {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };

                let addr = match addr.resolve_first().await {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };

                Some(Ok((buf, addr)))
            })
            .forward(upstream_sink.sink_map_err(anyhow::Error::from)),
    );

    let download_task = upstream_stream
        .map_ok(|(data, addr)| (data, Address::from(addr)))
        .forward(udp_sink);

    select! {
        _ = upload_task.fuse() => Ok(()),
        _ = download_task.fuse() => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::tcpman::udp_stream::{PacketReader, PacketWriter};
    use crate::rt::block_on;

    use crate::{
        test::{duplex, echo_udp_server},
        utils::read_bincode_lengthed_async,
    };

    use super::*;

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

            let mut reader = PacketReader::new();
            // Must have received echo-ed data

            let (payload, addr) = reader.read(&mut near).await?;
            assert_eq!(server_addr.port(), addr.get_port());
            assert_eq!(initial_data, payload.as_ref());

            let payload = b"second payload!";
            let mut writer = PacketWriter::new();
            writer
                .write(&mut near, &server_addr.into(), payload)
                .await?;

            let (data, addr) = reader.read(&mut near).await?;
            assert_eq!(addr, &server_addr.into());
            assert_eq!(payload, data.as_ref());

            Ok(())
        })
    }
}
