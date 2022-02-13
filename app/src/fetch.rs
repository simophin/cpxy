use std::{borrow::Cow, io::Write};

use anyhow::{bail, Context};
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{
    http::{AsyncHttpStream, HttpCommon, HttpRequest, HttpResponse, HttpUrl},
    io::TcpStream,
    socks5::Address,
    stream::AsyncReadWrite,
    tls::connect_tls,
    utils::RWBuffer,
};

async fn prepare_connection(
    proxy: Option<&Address>,
    is_ssl: bool,
    host: &str,
    port: u16,
) -> anyhow::Result<(impl AsyncRead + AsyncWrite + Unpin + Send + Sync, Vec<u8>)> {
    let mut buf = Vec::<u8>::new();
    let client = if let Some(proxy) = proxy {
        let mut c = TcpStream::connect(proxy)
            .await
            .context("Connecting to proxy server: {proxy}")?;

        write!(&mut buf, "CONNECT {host}:{port} HTTP/1.1\r\n\r\n")?;
        c.write_all(buf.as_ref())
            .await
            .context("HTTP Proxy CONNECT")?;
        buf.clear();

        let client = super::http::parse_response(c, Default::default())
            .await
            .context("Parse HTTP Proxy response")?;

        if client.status_code != 200 {
            bail!("Http proxy returns error code: {}", client.status_code);
        }

        if is_ssl {
            AsyncReadWrite::new(
                connect_tls(host, client)
                    .await
                    .with_context(|| format!("Connecting to TLS: {host}"))?,
            )
        } else {
            AsyncReadWrite::new(client)
        }
    } else {
        let client = TcpStream::connect_raw((host, port)).await?;
        if is_ssl {
            AsyncReadWrite::new(
                connect_tls(host, client)
                    .await
                    .with_context(|| format!("Connecting to TLS: {host}"))?,
            )
        } else {
            AsyncReadWrite::new(client)
        }
    };

    buf.clear();
    Ok((client, buf))
}

pub async fn send_http<'a>(
    req: HttpRequest<'a>,
    http_proxy: Option<&Address>,
) -> anyhow::Result<(
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    Vec<u8>,
)> {
    let (host, is_ssl, port) = match &req.url {
        HttpUrl::WithScheme { address, https, .. } => {
            (address.get_host(), *https, address.get_port())
        }
        _ => bail!("Invalid http request: {req:?}"),
    };
    let (mut client, mut buf) = prepare_connection(http_proxy, is_ssl, host.as_ref(), port).await?;

    req.to_writer(&mut buf)?;

    client.write_all(&buf).await?;
    buf.clear();
    Ok((client, buf))
}

pub async fn fetch_http<'a, 'b>(
    url: &'a str,
    method: &'a str,
    headers: impl Iterator<Item = (&'b str, Cow<'b, str>)> + Send + Sync + 'b,
    http_proxy: Option<&Address>,
    body: Option<(&str, &[u8])>,
) -> anyhow::Result<
    AsyncHttpStream<HttpResponse<'static>, impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'a>,
> {
    let mut headers = headers
        .map(|(k, v)| (Cow::Borrowed(k), v))
        .collect::<Vec<_>>();

    let body = if let Some((content_type, body)) = body {
        headers.push((Cow::Borrowed("Content-Type"), Cow::Borrowed(content_type)));
        headers.push((
            Cow::Borrowed("Content-Length"),
            Cow::Owned(format!("Content-Length: {}", body.len())),
        ));
        Some(body)
    } else {
        None
    };

    let (mut client, buf) = send_http(
        HttpRequest {
            url: url.parse()?,
            method: Cow::Borrowed(method),
            common: HttpCommon { headers },
        },
        http_proxy,
    )
    .await?;

    if let Some(body) = body {
        client.write_all(body).await?;
    }

    super::http::parse_response(client, RWBuffer::new(buf)).await
}
