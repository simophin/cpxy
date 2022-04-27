use async_stream::stream;
use bytes::Bytes;
use futures::{AsyncRead, AsyncWrite, Sink, SinkExt, Stream, StreamExt};

use super::udp_stream::{PacketReader, PacketWriter};
use crate::{
    rt::{mpsc::channel, spawn},
    socks5::Address,
};

pub fn create_udp_stream(
    mut r: impl AsyncRead + Unpin + Send,
) -> impl Stream<Item = (Bytes, Address<'static>)> + Unpin + Send {
    let mut reader = PacketReader::new();

    Box::pin(stream! {
        while let Ok((data, addr)) = reader.read(&mut r).await {
            yield ((data, addr.clone().into_owned()))
        }
    })
}

pub fn create_udp_sink(
    mut w: impl AsyncWrite + Unpin + Send + 'static,
) -> impl Sink<(Bytes, Address<'static>), Error = anyhow::Error> {
    let (tx, mut rx) = channel::<(Bytes, Address<'static>)>(10);
    spawn(async move {
        let mut writer = PacketWriter::new();
        while let Some((data, addr)) = rx.next().await {
            if let Err(e) = writer.write(&mut w, &addr, &data).await {
                log::error!("Erorr writing packet: {e:?}");
                break;
            }
        }
    })
    .detach();

    tx.sink_err_into()
}
