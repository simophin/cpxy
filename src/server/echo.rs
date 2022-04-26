use bytes::Bytes;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, SinkExt, StreamExt};

use crate::{
    proxy::{
        protocol::ProxyResult,
        udp_stream::{PacketReader, PacketWriter},
    },
    rt::{mpsc::channel, spawn, Task},
    utils::{new_vec_for_udp, write_bincode_lengthed_async, VecExt},
};

pub async fn serve_tcp(
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    write_bincode_lengthed_async(
        &mut stream,
        &ProxyResult::Granted {
            bound_address: None,
            solved_addresses: None,
        },
    )
    .await?;

    let (mut r, mut w) = stream.split();
    let (mut tx, mut rx) = channel::<Bytes>(2);

    let task: Task<anyhow::Result<()>> = spawn(async move {
        while let Some(buf) = rx.next().await {
            w.write_all(&buf).await?;
        }
        Ok(())
    });

    loop {
        let mut buf = new_vec_for_udp();
        let len = r.read(&mut buf).await?;
        if len == 0 {
            break;
        }
        buf.set_len_uninit(len);
        tx.send(buf.into()).await?;
    }

    task.await
}

pub async fn serve_udp(
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    write_bincode_lengthed_async(
        &mut stream,
        &ProxyResult::Granted {
            bound_address: None,
            solved_addresses: None,
        },
    )
    .await?;

    let (mut r, mut w) = stream.split();
    let mut packet_stream = PacketReader::new();

    // let (tx, mut rx) = bounded(10);
    // let task: Task<anyhow::Result<()>> = spawn(async move {

    // });

    let mut packet_writer = PacketWriter::new();
    loop {
        let (pkt, addr) = packet_stream.read(&mut r).await?;
        packet_writer.write(&mut w, &addr, pkt.as_ref()).await?;
    }
}
