use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    io::Write,
    ops::Deref,
    str::FromStr,
};

use anyhow::{anyhow, bail, Context};
use derive_more::Deref;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use pin_project_lite::pin_project;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::buf::RWBuffer;

pub enum HeaderValue<'a> {
    Str(Cow<'a, str>),
    Dis(Box<dyn Display + Send + Sync + 'a>),
}

impl<'a> From<&'a str> for HeaderValue<'a> {
    fn from(v: &'a str) -> Self {
        HeaderValue::Str(Cow::Borrowed(v))
    }
}

impl From<String> for HeaderValue<'_> {
    fn from(v: String) -> Self {
        HeaderValue::Str(Cow::Owned(v))
    }
}

impl<'a> HeaderValue<'a> {
    pub fn from_display(d: impl Send + Sync + Display + 'a) -> Self {
        HeaderValue::Dis(Box::new(d))
    }

    pub fn as_str(&'a self) -> Cow<'a, str> {
        match self {
            HeaderValue::Str(s) => Cow::Borrowed(s.as_ref()),
            HeaderValue::Dis(s) => Cow::Owned(s.to_string()),
        }
    }
}

impl<'a> FromStr for HeaderValue<'a> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(s.to_string().into())
    }
}

impl<'a> Display for HeaderValue<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeaderValue::Str(v) => f.write_str(v.as_ref()),
            HeaderValue::Dis(d) => Display::fmt(d.as_ref(), f),
        }
    }
}

impl<'a> Debug for HeaderValue<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl<'a> Serialize for HeaderValue<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'a, 'de> Deserialize<'de> for HeaderValue<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(<String as Deserialize>::deserialize(deserializer)?.into())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HttpCommon<'a> {
    pub headers: Vec<(Cow<'a, str>, HeaderValue<'a>)>,
}

#[derive(Debug, Serialize, Deserialize, Deref)]
pub struct HttpRequest<'a> {
    #[serde(flatten)]
    #[deref]
    pub common: HttpCommon<'a>,
    pub method: Cow<'a, str>,
    pub path: Cow<'a, str>,
}

#[derive(Debug, Deref)]
pub struct HttpResponse<'a> {
    #[deref]
    common: HttpCommon<'a>,
    pub status_code: u16,
}

impl<'a> HttpRequest<'a> {
    pub fn parse(buf: &[u8]) -> anyhow::Result<Option<(usize, Self)>>
    where
        'a: 'static,
    {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf).context("Pasring HTTP request")? {
            httparse::Status::Complete(offset) => {
                let method = req.method.ok_or_else(|| anyhow!("No method"))?;
                let path = req.path.ok_or_else(|| anyhow!("No path"))?.to_string();
                let headers = req
                    .headers
                    .iter()
                    .map(|hdr| {
                        (
                            Cow::Owned(hdr.name.to_string()),
                            String::from_utf8_lossy(hdr.value).to_string().into(),
                        )
                    })
                    .collect();

                Ok(Some((
                    offset,
                    HttpRequest {
                        path: Cow::Owned(path),
                        method: Cow::Owned(method.to_string()),
                        common: HttpCommon { headers },
                    },
                )))
            }
            httparse::Status::Partial => Ok(None),
        }
    }

    pub async fn to_async_writer(
        &self,
        w: &mut (impl AsyncWrite + Unpin + Send + Sync),
    ) -> anyhow::Result<()> {
        let mut line_buf = Vec::new();
        write!(&mut line_buf, "{} {} HTTP/1.1\r\n", self.method, self.path)?;
        w.write_all(&line_buf).await?;

        for (k, v) in &self.headers {
            line_buf.clear();
            write!(&mut line_buf, "{k}: {v}\r\n")?;
            w.write_all(&line_buf).await?;
        }

        w.write_all(b"\r\n").await?;
        Ok(())
    }
}

impl<'a> HttpCommon<'a> {
    pub fn get_header(&'a self, n: &str) -> Option<&HeaderValue<'a>> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v)
    }

    pub fn get_header_text(&'a self, n: &str) -> Option<Cow<'a, str>> {
        self.get_header(n).map(|v| v.as_str())
    }

    pub fn get_content_length(&self) -> Option<usize> {
        self.get_header_text("content-length")
            .and_then(|v| v.parse().ok())
    }

    pub fn get_content_type(&self) -> Option<Cow<str>> {
        self.get_header_text("content-type")
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

impl<'a, I: Deref<Target = HttpCommon<'a>>, T: AsyncRead + Unpin + Send + Sync>
    AsyncHttpStream<I, T>
{
    pub async fn body(&mut self) -> anyhow::Result<Vec<u8>> {
        match (
            self.deref().get_content_length(),
            self.deref().get_header_text("transfer-encoding"),
        ) {
            (Some(len), _) if len > 0 => {
                let mut buf = vec![0u8; len];
                self.read_exact(buf.as_mut_slice()).await?;
                Ok(buf)
            }
            (_, Some(e)) if e.eq_ignore_ascii_case("chunked") => {
                let mut final_buf = Vec::new();
                let mut buf = RWBuffer::new(8, 24);
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

    pub async fn body_json<R: DeserializeOwned>(&mut self) -> anyhow::Result<R> {
        match self.get_content_type() {
            Some(v) if v.to_ascii_lowercase().starts_with("application/json") => {}
            v => bail!("Invalid content type: {v:?}"),
        };

        Ok(serde_json::from_slice(self.body().await?.as_ref())?)
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
                            String::from_utf8_lossy(hdr.value).to_string().into(),
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
                        common: HttpCommon { headers },
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
                            String::from_utf8_lossy(hdr.value).to_string().into(),
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
                        common: HttpCommon { headers },
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
