use crate::handshake::ProxyRequest;
use crate::utils::{copy_io, RWBuffer};
use bytes::BufMut;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::io::Cursor;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::select;
use tokio::time::timeout;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ProxyResult {
    Granted { bound_address: SocketAddr },
    ErrHostNotFound,
    ErrTimeout,
    ErrGeneric { msg: String },
}

impl std::fmt::Display for ProxyResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for ProxyResult {}

fn write_json(buf: &mut RWBuffer, o: &impl Serialize) -> anyhow::Result<()> {
    let written = {
        let mut cursor = Cursor::new(&mut buf.write_buf()[2..]);
        serde_json::to_writer(&mut cursor, o)?;
        cursor.position() as usize
    };

    buf.write_buf().put_u16(written.try_into()?);
    buf.advance_write(2 + written);
    Ok(())
}

async fn write_json_async(
    w: &mut (impl AsyncWrite + Unpin),
    o: impl Serialize,
) -> anyhow::Result<()> {
    let data = serde_json::to_string(&o)?;
    w.write_u16(data.as_bytes().len().try_into()?).await?;
    w.write_all(data.as_bytes()).await?;
    Ok(())
}

async fn read_json_async<T: DeserializeOwned>(
    r: &mut (impl AsyncRead + Unpin),
) -> anyhow::Result<T> {
    let mut len_buf = [0u8; 2];
    r.read_exact(&mut len_buf).await?;
    let len = u16::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0; len];
    r.read_exact(buf.as_mut_slice()).await?;
    Ok(serde_json::from_slice(buf.as_slice())?)
}

pub async fn request_proxy<
    S: AsyncRead + AsyncWrite + Unpin,
    Fut: Future<Output = anyhow::Result<S>> + Send + Sync,
>(
    proxy_req: &ProxyRequest,
    connect_upstream: impl (FnOnce(RWBuffer) -> Fut) + Send + Sync,
) -> anyhow::Result<(SocketAddr, S)> {
    let mut req_buf = RWBuffer::default();
    log::info!("Sending request {proxy_req:?}");
    write_json(&mut req_buf, proxy_req)?;
    let mut upstream = connect_upstream(req_buf).await?;
    match read_json_async(&mut upstream).await? {
        ProxyResult::Granted { bound_address } => Ok((bound_address, upstream)),
        v => Err(v.into()),
    }
}

pub async fn serve_proxy<
    Stream: AsyncRead + AsyncWrite + Unpin,
    ResourceFut: Future<Output = anyhow::Result<(SocketAddr, Stream)>> + Send + Sync + 'static,
>(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    resource_fetcher: impl (FnOnce(ProxyRequest) -> ResourceFut) + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let req = read_json_async(&mut stream).await?;
    log::info!("Processing request {req:?}");
    let (res_r, res_w) = match timeout(Duration::from_secs(2), resource_fetcher(req)).await {
        Ok(Ok((bound_address, v))) => {
            write_json_async(&mut stream, ProxyResult::Granted { bound_address }).await?;
            split(v)
        }
        Ok(Err(e)) => {
            write_json_async(&mut stream, ProxyResult::ErrGeneric { msg: e.to_string() }).await?;
            return Err(e);
        }
        Err(e) => {
            write_json_async(&mut stream, ProxyResult::ErrTimeout).await?;
            return Err(e.into());
        }
    };

    let (r, w) = split(stream);

    select! {
        r1 = copy_io(r, res_w) => r1,
        r2 = copy_io(res_r, w) => r2,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use tokio::io::{duplex, AsyncWriteExt};
    use tokio::spawn;

    #[tokio::test]
    async fn test_proxy() {}
}
