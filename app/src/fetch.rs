use std::borrow::Cow;

use anyhow::Context;
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{
    http::{AsyncHttpStream, HttpCommon, HttpRequest, HttpResponse},
    io::TcpStream,
    socks5::Address,
    stream::AsyncReadWrite,
    tls::connect_tls,
    url::HttpUrl,
    utils::RWBuffer,
};

pub async fn send_http_with_proxy(
    https: bool,
    address: &Address<'_>,
    mut req: HttpRequest<'_>,
    http_proxy: &Address<'_>,
) -> anyhow::Result<(
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    Vec<u8>,
)> {
    let mut client = TcpStream::connect(http_proxy)
        .await
        .with_context(|| format!("Connecting to proxy server: {http_proxy}"))?;

    let scheme = if https { "https://" } else { "http://" };
    req.path = Cow::Owned(format!("{scheme}{address}{}", req.path));

    let mut buf = Vec::new();
    req.to_writer(&mut buf).context("Writing request headers")?;
    log::debug!(
        "Sending http (proxy = {http_proxy}): {}",
        String::from_utf8_lossy(&buf)
    );
    client.write_all(&buf).await?;

    Ok((client, buf))
}

pub async fn send_http(
    https: bool,
    address: &Address<'_>,
    req: HttpRequest<'_>,
) -> anyhow::Result<(
    impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    Vec<u8>,
)> {
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

    let mut buf = Vec::new();
    req.to_writer(&mut buf).context("Writing request headers")?;
    client.write_all(&buf).await?;

    Ok((client, buf))
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

    let HttpUrl {
        is_https,
        address,
        path,
    } = url.try_into()?;

    let (mut client, buf) = send_http_with_proxy(
        is_https,
        &address,
        HttpRequest {
            path,
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
