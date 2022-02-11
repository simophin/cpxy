use std::{borrow::Cow, io::Write};

use anyhow::{bail, Context};
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};
use url::Url;

use crate::{
    http::{AsyncHttpStream, HttpResponse},
    io::TcpStream,
    socks5::Address,
    stream::AsyncReadWrite,
    tls::connect_tls,
    utils::RWBuffer,
};

pub async fn fetch_http<'a, 'b>(
    url: &'a str,
    method: &'a str,
    headers: impl Iterator<Item = (&'b str, Cow<'b, str>)> + Send + Sync + 'b,
    http_proxy: &Address,
    body: Option<(&str, &[u8])>,
) -> anyhow::Result<
    AsyncHttpStream<HttpResponse, impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'a>,
> {
    let url = Url::parse(url)?;
    let (scheme, host, port, path) = match url.domain() {
        Some(host) => (url.scheme(), host, url.port(), url.path()),
        _ => bail!("No domain specified"),
    };

    let (is_ssl, port) = match scheme {
        s if s.eq_ignore_ascii_case("http") => (false, port.unwrap_or(80)),
        s if s.eq_ignore_ascii_case("https") => (true, port.unwrap_or(443)),
        s => bail!("Unknown scheme for fetching http: {s}"),
    };

    let mut buf = Vec::<u8>::new();

    let mut client = TcpStream::connect(http_proxy)
        .await
        .context("Connecting to http proxy")?;

    write!(&mut buf, "CONNECT {host}:{port} HTTP/1.1\r\n\r\n")?;
    client
        .write_all(buf.as_ref())
        .await
        .context("HTTP Proxy CONNECT")?;
    buf.clear();

    let client = super::http::parse_response(client, Default::default())
        .await
        .context("Parse HTTP Proxy response")?;

    if client.status_code != 200 {
        bail!("Http proxy returns error code: {}", client.status_code);
    }

    let mut client = if is_ssl {
        AsyncReadWrite::new(
            connect_tls(host, client)
                .await
                .with_context(|| format!("Connecting to TLS: {url}"))?,
        )
    } else {
        AsyncReadWrite::new(client)
    };

    write!(
        &mut buf,
        "{method} {path} HTTP/1.1\r\n\
        Host: {host}\r\n"
    )?;

    for (k, v) in headers {
        write!(&mut buf, "{k}: {v}\r\n")?;
    }

    let body = if let Some((content_type, body)) = body {
        write!(
            &mut buf,
            "Content-Type: {content_type}\r\n\
             Content-Length: {}\r\n",
            body.len()
        )?;
        Some(body)
    } else {
        None
    };

    write!(&mut buf, "\r\n")?;

    client.write_all(buf.as_ref()).await?;
    if let Some(body) = body {
        client.write_all(body).await?;
    }

    buf.clear();
    buf.resize(8192, 0);

    super::http::parse_response(client, RWBuffer::new(buf)).await
}
