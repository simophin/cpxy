use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use futures::io::{ReadHalf, WriteHalf};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, Future, FutureExt};

use crate::counter::Counter;
use crate::io::DatagramSocket;
use crate::rt::{spawn, Task};
use crate::utils::{copy_duplex, race};

pub async fn serve_stream_over_stream(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    upstream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    txc: Option<Arc<Counter>>,
    rxc: Option<Arc<Counter>>,
) -> anyhow::Result<()> {
    copy_duplex(stream, upstream, txc, rxc).await
}

pub async fn serve_dgram_over_stream<S, UploadFut, DownloadFut>(
    dgram: Arc<impl DatagramSocket<RecvType = (Bytes, SocketAddr)> + Unpin + Send + Sync + 'static>,
    upstream: S,
    mut dgram_writer: impl FnMut(&mut WriteHalf<S>, Bytes, SocketAddr) -> UploadFut
        + Send
        + Sync
        + 'static,
    mut dgram_reader: impl FnMut(&mut ReadHalf<S>) -> DownloadFut + Send + Sync + 'static,
    txc: Option<Arc<Counter>>,
    rxc: Option<Arc<Counter>>,
) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    UploadFut: Future<Output = anyhow::Result<usize>> + Send + Sync,
    DownloadFut: Future<Output = anyhow::Result<(Bytes, SocketAddr, usize)>> + Send + Sync,
{
    let (mut upstream_r, mut upstream_w) = upstream.split();
    let upload_task: Task<anyhow::Result<()>> = {
        let dgram = dgram.clone();
        let txc = txc.unwrap_or_default();
        spawn(async move {
            loop {
                let (data, dst) = dgram.recv_dgram().await?;
                let len = dgram_writer(&mut upstream_w, data, dst).await?;
                txc.inc(len);
            }
        })
    };

    let rxc = rxc.unwrap_or_default();
    let download_task: Task<anyhow::Result<()>> = spawn(async move {
        loop {
            let (data, dst, read_len) = dgram_reader(&mut upstream_r).await?;
            rxc.inc(read_len);
            dgram.send_dgram(&data, dst).await?;
        }
    });

    race(upload_task.fuse(), download_task.fuse()).await
}
