use std::{borrow::Cow, pin::Pin};

use anyhow::Context;
use futures::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{
    buf::RWBuffer,
    http::{AsyncHttpStream, HttpRequest, HttpResponse, WithHeaders},
    io::connect_tcp,
    rt::net::TcpStream,
    socks5::Address,
    tls::connect_tls,
    url::HttpUrl,
};

pub async fn send_http_with_proxy(
    https: bool,
    address: &Address<'_>,
    mut req: HttpRequest<'_>,
    http_proxy: &Address<'_>,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static> {
    let mut client = connect_tcp(http_proxy)
        .await
        .with_context(|| format!("Connecting to proxy server: {http_proxy}"))?;

    let scheme = if https { "https://" } else { "http://" };
    req.path = Cow::Owned(format!("{scheme}{address}{}", req.path));

    req.to_async_writer(&mut client)
        .await
        .context("Writing request headers")?;
    Ok(client)
}

pub enum HttpStream<T> {
    Plain(T),
    #[cfg(target_arch = "mips")]
    SSL(T),
    #[cfg(not(target_arch = "mips"))]
    SSL(futures_rustls::client::TlsStream<T>),
}

pub async fn connect_http_stream<T: AsyncRead + AsyncWrite + Unpin + Send + Sync>(
    tls: bool,
    address: &Address<'_>,
    client: T,
) -> anyhow::Result<HttpStream<T>> {
    let client = if tls {
        HttpStream::SSL(
            connect_tls(address.get_host().as_ref(), client)
                .await
                .context("TLS handshake")?,
        )
    } else {
        HttpStream::Plain(client)
    };
    Ok(client)
}

pub async fn connect_http(
    tls: bool,
    address: &Address<'_>,
) -> anyhow::Result<HttpStream<TcpStream>> {
    let client = connect_tcp(&address)
        .await
        .with_context(|| format!("Connecting to {address}"))?;

    client.set_nodelay(true).context("Enabling NO_DELAY")?;
    connect_http_stream(tls, address, client).await
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for HttpStream<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            HttpStream::Plain(s) => AsyncRead::poll_read(Pin::new(s), cx, buf),
            HttpStream::SSL(s) => AsyncRead::poll_read(Pin::new(s), cx, buf),
        }
    }

    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            HttpStream::Plain(s) => AsyncRead::poll_read_vectored(Pin::new(s), cx, bufs),
            HttpStream::SSL(s) => AsyncRead::poll_read_vectored(Pin::new(s), cx, bufs),
        }
    }
}

impl<T> HttpStream<T> {
    pub fn inner(&self) -> &T {
        match self {
            HttpStream::Plain(stream) => stream,
            #[cfg(not(target_arch = "mips"))]
            HttpStream::SSL(stream) => stream.get_ref().0,
            #[cfg(target_arch = "mips")]
            HttpStream::SSL(stream) => stream,
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for HttpStream<T> {
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            HttpStream::Plain(s) => AsyncWrite::poll_write_vectored(Pin::new(s), cx, bufs),
            HttpStream::SSL(s) => AsyncWrite::poll_write_vectored(Pin::new(s), cx, bufs),
        }
    }

    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            HttpStream::Plain(s) => AsyncWrite::poll_write(Pin::new(s), cx, buf),
            HttpStream::SSL(s) => AsyncWrite::poll_write(Pin::new(s), cx, buf),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            HttpStream::Plain(s) => AsyncWrite::poll_flush(Pin::new(s), cx),
            HttpStream::SSL(s) => AsyncWrite::poll_flush(Pin::new(s), cx),
        }
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            HttpStream::Plain(s) => AsyncWrite::poll_flush(Pin::new(s), cx),
            HttpStream::SSL(s) => AsyncWrite::poll_flush(Pin::new(s), cx),
        }
    }
}

pub async fn send_http(
    https: bool,
    address: &Address<'_>,
    req: &HttpRequest<'_>,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static> {
    let mut stream = connect_http(https, address).await?;
    req.to_async_writer(&mut stream).await?;
    Ok(stream)
}

pub async fn fetch_http_with_proxy<'a, 'b>(
    url: &'a str,
    method: &'a str,
    headers: impl Iterator<Item = (Cow<'b, str>, Cow<'b, [u8]>)> + Send + Sync + 'b,
    http_proxy: &Address<'_>,
    body: Option<(&str, &[u8])>,
) -> anyhow::Result<
    AsyncHttpStream<HttpResponse<'static>, impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'a>,
> {
    let mut headers = headers.collect::<Vec<_>>();

    let body = if let Some((content_type, body)) = body {
        headers.push((
            Cow::Borrowed("Content-Type"),
            Cow::Borrowed(content_type.as_bytes()),
        ));
        headers.push((
            Cow::Borrowed("Content-Length"),
            Cow::Owned(body.len().to_string().into_bytes()),
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

    let mut request = HttpRequest {
        path,
        method: Cow::Borrowed(method),
        headers,
    };

    if request.get_header("host").is_none() {
        request.headers.push((
            Cow::Borrowed("Host"),
            Cow::Owned(address.to_string().into_bytes()),
        ));
    }

    let mut client = send_http_with_proxy(is_https, &address, request, http_proxy).await?;

    if let Some(body) = body {
        client.write_all(body).await?;
    }

    super::http::parse_response(client, RWBuffer::new_vec_uninitialised(512)).await
}
