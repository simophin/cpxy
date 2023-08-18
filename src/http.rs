use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    io::Write,
    ops::Deref,
};

use anyhow::{anyhow, bail, Context};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use pin_project_lite::pin_project;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::buf::RWBuffer;

pub trait WithHeaders {
    fn headers(&self) -> &Vec<(Cow<str>, Cow<[u8]>)>;

    fn get_header(&self, n: &str) -> Option<&[u8]> {
        for (k, v) in self.headers() {
            if k.eq_ignore_ascii_case(n) {
                return Some(v.as_ref());
            }
        }
        None
    }

    fn get_header_text(&self, n: &str) -> Option<&str> {
        self.get_header(n).and_then(|v| std::str::from_utf8(v).ok())
    }

    fn get_content_length(&self) -> Option<usize> {
        self.get_header_text("content-length")
            .and_then(|v| v.parse().ok())
    }

    fn get_content_type(&self) -> Option<&str> {
        self.get_header_text("content-type")
    }
}

pub struct HttpRequestBuilder {
    buf: Vec<u8>,
}

impl HttpRequestBuilder {
    pub fn new(method: impl Display, path: impl Display) -> anyhow::Result<Self> {
        let mut buf = vec![];
        write!(&mut buf, "{method} {path} HTTP/1.1\r\n")?;
        Ok(Self { buf })
    }

    pub fn put_header_text(
        &mut self,
        name: impl Display,
        value: impl Display,
    ) -> anyhow::Result<&mut Self> {
        write!(&mut self.buf, "{name}: {value}\r\n")?;
        Ok(self)
    }

    pub fn put_header(&mut self, name: impl Display, value: &[u8]) -> anyhow::Result<&mut Self> {
        write!(&mut self.buf, "{name}: ")?;
        self.buf.extend_from_slice(value);
        self.buf.extend_from_slice(b"\r\n");
        Ok(self)
    }

    pub fn finalise(self) -> Vec<u8> {
        let Self { mut buf } = self;
        buf.extend_from_slice(b"\r\n");
        buf
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HttpRequest<'a> {
    pub headers: Vec<(Cow<'a, str>, Cow<'a, [u8]>)>,
    pub method: Cow<'a, str>,
    pub path: Cow<'a, str>,
}

#[derive(Debug)]
pub struct HttpResponse<'a> {
    pub headers: Vec<(Cow<'a, str>, Cow<'a, [u8]>)>,
    pub status_code: u16,
}

impl<'a> WithHeaders for HttpRequest<'a> {
    fn headers(&self) -> &Vec<(Cow<str>, Cow<[u8]>)> {
        &self.headers
    }
}

impl<'a> WithHeaders for HttpResponse<'a> {
    fn headers(&self) -> &Vec<(Cow<str>, Cow<[u8]>)> {
        &self.headers
    }
}

impl<'a> HttpRequest<'a> {
    pub fn into_owned(self) -> HttpRequest<'static> {
        let Self {
            headers,
            method,
            path,
        } = self;
        HttpRequest {
            method: Cow::Owned(method.into_owned()),
            path: Cow::Owned(path.into_owned()),
            headers: headers
                .into_iter()
                .map(|(k, v)| (Cow::Owned(k.into_owned()), Cow::Owned(v.into_owned())))
                .collect(),
        }
    }

    pub fn parse(buf: &'a [u8]) -> anyhow::Result<Option<(usize, Self)>> {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf).context("Pasring HTTP request")? {
            httparse::Status::Complete(offset) => {
                let method = req.method.ok_or_else(|| anyhow!("No method"))?;
                let path = req.path.ok_or_else(|| anyhow!("No path"))?;
                let headers = req
                    .headers
                    .iter()
                    .map(|hdr| (Cow::Borrowed(hdr.name), Cow::Borrowed(hdr.value)))
                    .collect();

                Ok(Some((
                    offset,
                    HttpRequest {
                        path: Cow::Borrowed(path),
                        method: Cow::Borrowed(method),
                        headers,
                    },
                )))
            }
            httparse::Status::Partial => Ok(None),
        }
    }

    pub fn to_builder(&self) -> HttpRequestBuilder {
        let mut builder = HttpRequestBuilder::new(&self.method, &self.path).unwrap();
        for (k, v) in &self.headers {
            builder.put_header(k, v.as_ref()).unwrap();
        }
        builder
    }

    pub async fn to_async_writer(
        &self,
        w: &mut (impl AsyncWrite + Unpin + Send + Sync),
    ) -> anyhow::Result<()> {
        w.write_all(&self.to_builder().finalise()).await?;
        Ok(())
    }
}

pin_project! {
    pub struct AsyncHttpStream<I, T> {
        init: I,

        body_buf: Option<RWBuffer>,

        #[pin]
        body: T
    }
}

impl<I: Debug, T> Debug for AsyncHttpStream<I, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncHttpStream")
            .field("init", &self.init)
            .finish()
    }
}

impl<I: WithHeaders, T: AsyncRead + Unpin + Send + Sync> AsyncHttpStream<I, T> {
    pub async fn body(&mut self) -> anyhow::Result<Vec<u8>> {
        let content_len = self.init.get_content_length();
        let transfer_encoding = self.init.get_header_text("transfer-encoding");
        match (content_len, transfer_encoding) {
            (Some(len), e) if len > 0 => {
                let mut buf = vec![0u8; len];
                drop(e);
                self.read_exact(buf.as_mut_slice()).await?;
                Ok(buf)
            }
            (_, Some(e)) if e.eq_ignore_ascii_case("chunked") => {
                let mut final_buf = Vec::new();
                let mut buf = RWBuffer::new_vec_uninitialised(8);
                loop {
                    match self.read(buf.write_buf()).await? {
                        0 => return Ok(final_buf),
                        v => buf.advance_write(v),
                    };

                    match httparse::parse_chunk_size(buf.read_buf()) {
                        Ok(httparse::Status::Complete((offset, len))) if len > 0 => {
                            buf.advance_read(offset);
                            final_buf.extend_from_slice(buf.read_buf());
                            let len = len as usize - buf.remaining_read();
                            let buf_start = final_buf.len();
                            final_buf.resize(buf_start + len, 0);
                            self.read_exact(&mut final_buf.as_mut_slice()[buf_start..])
                                .await?;
                            buf.advance_read(buf.remaining_read());
                        }
                        Ok(httparse::Status::Complete(_)) => return Ok(final_buf),
                        Ok(_) => continue,
                        Err(_) => bail!(
                            "Invalid chunk size on buf: {}",
                            String::from_utf8_lossy(buf.read_buf())
                        ),
                    }
                }
            }
            _ => Ok(Default::default()),
        }
    }

    pub async fn body_json_or_yaml<R: DeserializeOwned>(&mut self) -> anyhow::Result<R> {
        match self.get_content_type() {
            Some(v) if v.to_ascii_lowercase().starts_with("application/json") => {
                Ok(serde_json::from_slice(self.body().await?.as_ref())?)
            }

            Some(v) if v.to_ascii_lowercase().starts_with("text/yaml") => {
                Ok(serde_yaml::from_slice(self.body().await?.as_ref())?)
            }
            v => bail!("Invalid content type: {v:?}"),
        }
    }
}

impl<I, T> Deref for AsyncHttpStream<I, T> {
    type Target = I;

    fn deref(&self) -> &Self::Target {
        &self.init
    }
}

impl<I, T: AsyncRead> AsyncRead for AsyncHttpStream<I, T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let mut this = self.project();
        if let Some(existing) = &mut this.body_buf {
            let copy_len = buf.len().min(existing.remaining_read());
            (&mut buf[..copy_len]).copy_from_slice(&existing.read_buf()[..copy_len]);
            existing.advance_read(copy_len);
            if existing.remaining_read() == 0 {
                *this.body_buf = None;
            }
            cx.waker().wake_by_ref();
            return std::task::Poll::Ready(Ok(copy_len));
        }

        this.body.as_mut().poll_read(cx, buf)
    }
}

impl<I, T: AsyncWrite> AsyncWrite for AsyncHttpStream<I, T> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write(self.project().body, cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        AsyncWrite::poll_flush(self.project().body, cx)
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        AsyncWrite::poll_close(self.project().body, cx)
    }

    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write_vectored(self.project().body, cx, bufs)
    }
}

pub async fn parse_response<T: AsyncRead + Unpin + Send + Sync>(
    mut stream: T,
    mut buf: RWBuffer,
) -> anyhow::Result<AsyncHttpStream<HttpResponse<'static>, T>> {
    loop {
        match stream.read(buf.write_buf()).await? {
            0 => bail!(
                "Unexpected EOF while parsing HTTP response: write_buf_len = {}",
                buf.remaining_write()
            ),
            v => buf.advance_write(v),
        }

        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut response = httparse::Response::new(&mut headers);

        match response.parse(buf.read_buf())? {
            httparse::Status::Complete(offset) => {
                let status_code = response.code.ok_or_else(|| anyhow!("No status code"))?;
                let headers = response
                    .headers
                    .iter()
                    .map(|hdr| {
                        (
                            Cow::Owned(hdr.name.to_string()),
                            Cow::Owned(hdr.value.to_owned()),
                        )
                    })
                    .collect();

                drop(response);
                buf.advance_read(offset);
                let body_buf = if buf.remaining_read() > 0 {
                    Some(buf)
                } else {
                    None
                };

                return Ok(AsyncHttpStream {
                    init: HttpResponse {
                        headers,
                        status_code,
                    },
                    body_buf,
                    body: stream,
                });
            }
            httparse::Status::Partial => continue,
        };
    }
}

pub async fn parse_request<T: AsyncRead + Unpin + Send + Sync>(
    mut stream: T,
    mut buf: RWBuffer,
) -> Result<AsyncHttpStream<HttpRequest<'static>, T>, (anyhow::Error, T)> {
    loop {
        match stream.read(buf.write_buf()).await {
            Ok(0) => return Err((anyhow!("Unexpected EOF"), stream)),
            Err(e) => return Err((e.into(), stream)),
            Ok(v) => buf.advance_write(v),
        }

        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf.read_buf()) {
            Ok(httparse::Status::Complete(offset)) => {
                let method = req.method.unwrap_or_default().to_string();
                let path = req.path.unwrap_or_default().to_string();
                let headers = req
                    .headers
                    .iter()
                    .map(|hdr| {
                        (
                            Cow::Owned(hdr.name.to_string()),
                            Cow::Owned(hdr.value.to_owned()),
                        )
                    })
                    .collect();

                drop(req);
                buf.advance_read(offset);
                let body_buf = if buf.remaining_read() > 0 {
                    Some(buf)
                } else {
                    None
                };

                return Ok(AsyncHttpStream {
                    init: HttpRequest {
                        headers,
                        method: Cow::Owned(method),
                        path: Cow::Owned(path),
                    },
                    body_buf,
                    body: stream,
                });
            }
            Ok(httparse::Status::Partial) => continue,
            Err(e) => return Err((e.into(), stream)),
        };
    }
}

pub async fn write_http_response(
    w: &mut (impl AsyncWrite + Unpin + Send + Sync),
    code: u16,
    status_msg: Option<&str>,
    content_type: Option<&str>,
    body: &[u8],
) -> anyhow::Result<()> {
    w.write_all(
        format!(
            "HTTP/1.1 {code}{}\r\n\
    Content-Type: {}\r\n\
    Content-Length: {}\r\n\
    Access-Control-Allow-Headers: *\r\n\
    Access-Control-Allow-Origin: *\r\n\
    Access-Control-Allow-Methods: *\r\n\
    \r\n",
            status_msg.map(|m| format!(" {m}")).unwrap_or(String::new()),
            content_type.unwrap_or("application/octet-stream"),
            body.len()
        )
        .as_bytes(),
    )
    .await?;
    if !body.is_empty() {
        w.write_all(body).await?;
    }
    Ok(())
}
