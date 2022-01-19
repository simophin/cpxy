use crate::socks5::Address;
use crate::utils::{HttpRequest, RWBuffer};
use bytes::BufMut;
use serde::de::DeserializeOwned;
use serde_derive::{Deserialize, Serialize};
use std::future::Future;
use std::io::Cursor;
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ProxyRequest {
    SocksTCP(Address),
    SocksUDP(Address),
    Http(HttpRequest),
}

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

fn write_json(buf: &mut RWBuffer, o: &impl serde::Serialize) -> anyhow::Result<()> {
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
    o: impl serde::Serialize,
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
) -> anyhow::Result<(ProxyResult, S)> {
    let mut req_buf = RWBuffer::default();
    log::info!("Sending request {proxy_req:?}");
    write_json(&mut req_buf, proxy_req)?;
    let mut upstream = connect_upstream(req_buf).await?;
    Ok((read_json_async(&mut upstream).await?, upstream))
}

pub async fn receive_proxy_request(
    stream: &mut (impl AsyncRead + Unpin),
) -> anyhow::Result<ProxyRequest> {
    read_json_async(stream).await
}

pub async fn send_proxy_result(
    stream: &mut (impl AsyncWrite + Unpin),
    res: ProxyResult,
) -> anyhow::Result<()> {
    write_json_async(stream, res).await
}
