use futures_lite::{io::split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};

use crate::{
    buf::Buf,
    proxy::{
        protocol::ProxyResult,
        udp::{Packet, PacketWriter},
    },
    rt::{mpsc::bounded, spawn, Task},
    utils::write_bincode_lengthed_async,
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

    let (mut r, mut w) = split(stream);
    let (tx, mut rx) = bounded::<Buf>(2);

    let task: Task<anyhow::Result<()>> = spawn(async move {
        while let Some(buf) = rx.next().await {
            w.write_all(&buf).await?;
        }
        Ok(())
    });

    loop {
        let mut buf = Buf::new_for_udp();
        let len = r.read(&mut buf).await?;
        if len == 0 {
            break;
        }
        buf.set_len(len);
        tx.send(buf).await?;
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

    let (r, mut w) = split(stream);
    let mut packet_stream = Packet::new_packet_stream(r, None);

    // let (tx, mut rx) = bounded(10);
    // let task: Task<anyhow::Result<()>> = spawn(async move {

    // });

    let mut packet_writer = PacketWriter::new();
    while let Some((pkt, addr)) = packet_stream.next().await {
        packet_writer.write(&mut w, &addr, pkt.as_ref()).await?;
    }

    Ok(())
}
