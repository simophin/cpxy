use async_stream::stream;
use bytes::Bytes;
use futures::{channel::mpsc::channel, AsyncRead, AsyncWrite, Sink, SinkExt, Stream, StreamExt};
use tokio::spawn;

use super::udp_stream::{PacketReader, PacketWriter};
use crate::socks5::Address;

pub fn create_udp_stream(
    mut r: impl AsyncRead + Unpin + Send,
    initial_address: Option<Address<'_>>,
) -> impl Stream<Item = anyhow::Result<(Bytes, Address<'static>)>> + Unpin + Send {
    let mut reader = match initial_address {
        Some(a) => PacketReader::new_with_initial_addr(a.into_owned()),
        None => PacketReader::new(),
    };

    Box::pin(stream! {
        loop {
            let r = reader.read(&mut r).await.map_err(anyhow::Error::from);
            let is_error = r.is_err();
            yield r.map(|(data, addr)| (data, addr.clone().into_owned()));
            if is_error {
                break
            }
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
                log::error!("Error writing packet: {e:?}");
                break;
            }
        }
    })
    .detach();

    tx.sink_err_into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::echo_tcp_server;
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn udp_sink_stream_works() {
        let (_task, addr) = echo_tcp_server().await;
        let (r, w) = TcpStream::connect(addr).await.expect("To connect").split();
        let mut stream = create_udp_stream(r, None);
        let mut sink = create_udp_sink(w);

        let data = Bytes::from_static(b"hello, world");
        let addr: Address = "localhost:53".parse().unwrap();
        sink.send((data.clone(), addr.clone().into_owned()))
            .await
            .unwrap();

        let received = stream.next().await.unwrap().unwrap();
        assert_eq!(&data, &received.0);
        assert_eq!(&addr, &received.1);
    }
}
