use futures_lite::{io::split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};

use crate::{
    buf::Buf,
    proxy::{
        protocol::ProxyResult,
        udp::{Packet, PacketWriter},
    },
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

    let mut buf = Buf::new_with_len(4096, 4096);
    loop {
        let len = stream.read(&mut buf).await?;
        stream.write_all(&buf[..len]).await?;
    }
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
    let mut packet_writer = PacketWriter::new();
    while let Some((pkt, addr)) = packet_stream.next().await {
        packet_writer.write(&mut w, &addr, pkt.as_ref()).await?;
    }

    Ok(())
}
