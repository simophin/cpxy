use std::ops::Deref;

use anyhow::{anyhow, bail};
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite};
use pin_project_lite::pin_project;

use crate::utils::RWBuffer;

pub struct HttpCommon {
    pub headers: Vec<(String, String)>,
}

pub struct HttpRequest {
    common: HttpCommon,
    pub method: String,
    pub path: String,
}

pub struct HttpResponse {
    common: HttpCommon,
    pub status_code: u16,
}

impl Deref for HttpRequest {
    type Target = HttpCommon;

    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl Deref for HttpResponse {
    type Target = HttpCommon;

    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl HttpCommon {
    pub fn get_header(&self, n: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.as_str())
    }

    pub fn get_content_length(&self) -> Option<usize> {
        self.get_header("content-length")
            .and_then(|v| v.parse().ok())
    }

    pub fn get_content_type(&self) -> Option<&str> {
        self.get_header("content-type")
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

impl<I, T> AsyncHttpStream<I, T> {
    pub fn into_init(self) -> I {
        self.init
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

        AsyncRead::poll_read(this.body.as_mut(), cx, buf)
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

pub async fn parse_response(
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    mut buf: RWBuffer,
) -> anyhow::Result<AsyncHttpStream<HttpResponse, impl AsyncRead + AsyncWrite + Unpin + Send + Sync>>
{
    loop {
        match stream.read(buf.write_buf()).await? {
            0 => bail!("Unexpected EOF"),
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
                            hdr.name.to_string(),
                            String::from_utf8_lossy(hdr.value).to_string(),
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
) -> anyhow::Result<AsyncHttpStream<HttpRequest, T>> {
    loop {
        match stream.read(buf.write_buf()).await? {
            0 => bail!("Unexpected EOF"),
            v => buf.advance_write(v),
        }

        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf.read_buf())? {
            httparse::Status::Complete(offset) => {
                let method = req
                    .method
                    .ok_or_else(|| anyhow!("No method"))?
                    .to_ascii_lowercase();
                let path = req.path.ok_or_else(|| anyhow!("No path"))?.to_string();
                let headers = req
                    .headers
                    .iter()
                    .map(|hdr| {
                        (
                            hdr.name.to_string(),
                            String::from_utf8_lossy(hdr.value).to_string(),
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
                        method,
                        path,
                    },
                    body_buf,
                    body: stream,
                });
            }
            httparse::Status::Partial => continue,
        };
    }
}
