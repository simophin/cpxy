use crate::proxy::handler::{send_proxy_result, ProxyResult};
use crate::utils::{copy_duplex, HttpRequest};
use anyhow::anyhow;
use bytes::BufMut;
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};
use smol::net::TcpStream;
use std::io::Write;
use std::net::SocketAddr;
use url::Url;

async fn prepare(
    HttpRequest {
        method,
        path,
        mut headers,
    }: HttpRequest,
) -> anyhow::Result<(
    SocketAddr,
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
)> {
    let url = match Url::parse(&path) {
        Ok(v) if v.scheme().eq_ignore_ascii_case("http") && v.has_host() => v,
        Ok(v) => {
            return Err(anyhow!(
                "Invalid scheme({:?}) or host({:?})",
                v.scheme(),
                v.host()
            ));
        }
        Err(_) => {
            return Err(anyhow!("Invalid path {path}"));
        }
    };

    log::info!("Connecting to {url}");
    if headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("host"))
        .is_none()
        && url.domain().is_some()
    {
        headers.push(("Host".to_string(), url.domain().unwrap().to_string()));
    }

    let addr = format!(
        "{}:{}",
        url.host_str().unwrap(),
        url.port_or_known_default().unwrap_or(80)
    );

    let path = &path["http://".len()..];
    let path = match path.find("/") {
        Some(v) if v + 1 < path.len() => &path[v + 1..],
        _ => "/",
    };

    let mut upstream = TcpStream::connect(addr).await?;
    let mut request_buf: Vec<u8> = Vec::new();

    write!(&mut request_buf, "{method} {path} HTTP/1.1\r\n")?;
    for (hdr_name, hdr_value) in headers.into_iter() {
        request_buf.put_slice(hdr_name.as_bytes());
        request_buf.put_slice(b": ");
        request_buf.put_slice(hdr_value.as_bytes());
        request_buf.put_slice(b"\r\n");
    }
    request_buf.put_slice(b"\r\n");
    upstream.write_all(request_buf.as_slice()).await?;

    Ok((upstream.local_addr()?, upstream))
}

pub async fn serve_http_proxy(
    req: HttpRequest,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let upstream = match prepare(req).await {
        Ok((addr, v)) => {
            send_proxy_result(
                &mut stream,
                ProxyResult::Granted {
                    bound_address: addr,
                },
            )
            .await?;
            v
        }
        Err(e) => {
            send_proxy_result(&mut stream, ProxyResult::ErrGeneric { msg: e.to_string() }).await?;
            return Err(e);
        }
    };

    copy_duplex(upstream, stream).await
}
