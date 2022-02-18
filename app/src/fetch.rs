use std::borrow::Cow;

use anyhow::Context;
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{
    buf::RWBuffer,
    http::{AsyncHttpStream, HeaderValue, HttpCommon, HttpRequest, HttpResponse},
    io::TcpStream,
    socks5::Address,
    stream::AsyncReadWrite,
    tls::connect_tls,
    url::HttpUrl,
};

pub async fn send_http_with_proxy(
    https: bool,
    address: &Address<'_>,
    mut req: HttpRequest<'_>,
    http_proxy: &Address<'_>,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static> {
    let mut client = TcpStream::connect(http_proxy)
        .await
        .with_context(|| format!("Connecting to proxy server: {http_proxy}"))?;

    let scheme = if https { "https://" } else { "http://" };
    req.path = Cow::Owned(format!("{scheme}{address}{}", req.path));

    req.to_async_writer(&mut client)
        .await
        .context("Writing request headers")?;
    Ok(client)
}

pub async fn send_http(
    https: bool,
    address: &Address<'_>,
    req: HttpRequest<'_>,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static> {
    let client = TcpStream::connect(&address)
        .await
        .with_context(|| format!("Connecting to {address}"))?;

    let mut client = if https {
        AsyncReadWrite::new(
            connect_tls(address.get_host().as_ref(), client)
                .await
                .context("TLS handshake")?,
        )
    } else {
        AsyncReadWrite::new(client)
    };

    req.to_async_writer(&mut client)
        .await
        .context("Writing request headers")?;

    Ok(client)
}

pub async fn fetch_http_with_proxy<'a, 'b>(
    url: &'a str,
    method: &'a str,
    headers: impl Iterator<Item = (&'b str, Cow<'b, str>)> + Send + Sync + 'b,
    http_proxy: &Address<'_>,
    body: Option<(&str, &[u8])>,
) -> anyhow::Result<
    AsyncHttpStream<HttpResponse<'static>, impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'a>,
> {
    let mut headers = headers
        .map(|(k, v)| (Cow::Borrowed(k), HeaderValue::Str(v)))
        .collect::<Vec<_>>();

    let body = if let Some((content_type, body)) = body {
        headers.push((Cow::Borrowed("Content-Type"), content_type.into()));
        headers.push((
            Cow::Borrowed("Content-Length"),
            HeaderValue::from_display(body.len()),
        ));
        Some(body)
    } else {
        None
    };

    let HttpUrl {
        is_https,
        address,
        path,
    } = url.try_into()?;

    let mut common = HttpCommon { headers };
    if common.get_header("host").is_none() {
        common
            .headers
            .push((Cow::Borrowed("Host"), HeaderValue::from_display(&address)));
    }

    let mut client = send_http_with_proxy(
        is_https,
        &address,
        HttpRequest {
            path,
            method: Cow::Borrowed(method),
            common,
        },
        http_proxy,
    )
    .await?;

    if let Some(body) = body {
        client.write_all(body).await?;
    }

    super::http::parse_response(client, RWBuffer::new(512, 65536)).await
}
