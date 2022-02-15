use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    net::{IpAddr, SocketAddr},
    ops::Deref,
    str::FromStr,
};

use anyhow::{anyhow, bail};
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use pin_project_lite::pin_project;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};

use crate::{socks5::Address, utils::RWBuffer};

#[derive(Debug, PartialEq, Eq, SerializeDisplay, DeserializeFromStr)]
pub enum HttpUrl {
    PathOnly {
        path: String,
    },
    WithAddress {
        address: Address,
        path: String,
    },
    WithScheme {
        https: bool,
        address: Address,
        path: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HttpCommon<'a> {
    pub headers: Vec<(Cow<'a, str>, Cow<'a, str>)>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HttpRequest<'a> {
    #[serde(flatten)]
    pub common: HttpCommon<'a>,
    pub method: Cow<'a, str>,
    pub url: HttpUrl,
}

#[derive(Debug)]
pub struct HttpResponse<'a> {
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
        match req.parse(buf) {
            Ok(httparse::Status::Complete(offset)) => {
                let method = req
                    .method
                    .ok_or_else(|| anyhow!("No method"))?
                    .to_ascii_lowercase();
                let path = req.path.ok_or_else(|| anyhow!("No path"))?;
                let headers = req
                    .headers
                    .iter()
                    .map(|hdr| {
                        (
                            Cow::Owned(hdr.name.to_string()),
                            Cow::Owned(String::from_utf8_lossy(hdr.value).to_string()),
                        )
                    })
                    .collect();

                Ok(Some((
                    offset,
                    HttpRequest {
                        url: path.parse()?,
                        method: Cow::Owned(method),
                        common: HttpCommon { headers },
                    },
                )))
            }
            Ok(httparse::Status::Partial) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn to_writer(&self, buf: &mut impl std::io::Write) -> anyhow::Result<()> {
        write!(buf, "{} {} HTTP/1.1\r\n", self.method, self.url)?;
        for (k, v) in &self.headers {
            write!(buf, "{k}: {v}\r\n")?;
        }

        write!(buf, "\r\n")?;
        Ok(())
    }
}

impl<'a> Deref for HttpRequest<'a> {
    type Target = HttpCommon<'a>;

    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl<'a> Deref for HttpResponse<'a> {
    type Target = HttpCommon<'a>;

    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl<'a> HttpCommon<'a> {
    pub fn get_header(&self, n: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.as_ref())
    }

    pub fn get_content_length(&self) -> Option<usize> {
        self.get_header("content-length")
            .and_then(|v| v.parse().ok())
    }

    pub fn get_content_type(&self) -> Option<&str> {
        self.get_header("content-type")
    }
}

impl HttpUrl {
    pub fn path(&self) -> &str {
        match self {
            HttpUrl::PathOnly { path } => path.as_str(),
            HttpUrl::WithAddress { path, .. } => path.as_str(),
            HttpUrl::WithScheme { path, .. } => path.as_str(),
        }
    }
}

fn sanitise_path_for_url(p: &str) -> &str {
    if p == "/" {
        return "";
    }
    p
}

impl Display for HttpUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (scheme, address, path) = match self {
            Self::PathOnly { path } => return f.write_str(path.as_str()),
            Self::WithAddress { address, path } => ("", address, sanitise_path_for_url(&path)),
            Self::WithScheme {
                https,
                address,
                path,
            } if *https => ("https://", address, sanitise_path_for_url(&path)),
            Self::WithScheme { address, path, .. } => {
                ("http://", address, sanitise_path_for_url(&path))
            }
        };

        f.write_fmt(format_args!("{scheme}{address}{path}"))
    }
}

impl FromStr for HttpUrl {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let url = s.trim();
        let (is_https, url) = if url.starts_with('/') {
            return Ok(HttpUrl::PathOnly {
                path: url.to_string(),
            });
        } else if url.starts_with(HTTP_SCHEME) {
            (Some(false), &url[HTTP_SCHEME.len()..])
        } else if url.starts_with(HTTPS_SCHEME) {
            (Some(true), &url[HTTPS_SCHEME.len()..])
        } else {
            (None, url)
        };

        let (host_and_port, path) = match url.find('/') {
            Some(u) => (&url[..u], &url[u..]),
            None => (url, "/"),
        };

        let (host, port) = match host_and_port.rfind(':') {
            Some(u) if u < host_and_port.len() - 1 => (
                &host_and_port[..u],
                (&host_and_port[(u + 1)..])
                    .parse()
                    .map_err(|e| anyhow!("Parsing port number: {host_and_port}: {e:?}"))?,
            ),
            Some(_) => bail!("No port specified"),
            None if matches!(is_https, Some(true)) => (host_and_port, 443),
            None if matches!(is_https, Some(false)) => (host_and_port, 80),
            None => bail!("A URL without scheme must have port specified: {host_and_port}"),
        };

        let address = match IpAddr::from_str(host) {
            Ok(a) => Address::IP(SocketAddr::new(a, port)),
            Err(_) => Address::Name {
                host: host.to_string(),
                port,
            },
        };

        let path = path.to_string();
        if is_https.is_some() {
            Ok(HttpUrl::WithScheme {
                https: is_https.unwrap(),
                address,
                path,
            })
        } else {
            Ok(HttpUrl::WithAddress { address, path })
        }
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
            self.deref().get_header("transfer-encoding"),
        ) {
            (Some(len), _) if len > 0 => {
                let mut buf = vec![0u8; len];
                self.read_exact(buf.as_mut_slice()).await?;
                Ok(buf)
            }
            (_, Some(e)) if e.eq_ignore_ascii_case("chunked") => {
                let mut final_buf = Vec::new();
                let mut buf = RWBuffer::with_capacity(128);
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
                            Cow::Owned(hdr.name.to_string()),
                            Cow::Owned(String::from_utf8_lossy(hdr.value).to_string()),
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
                let method = req.method.unwrap_or_default().to_ascii_lowercase();
                let path = req.path.unwrap_or_default().to_string();
                let headers = req
                    .headers
                    .iter()
                    .map(|hdr| {
                        (
                            Cow::Owned(hdr.name.to_string()),
                            Cow::Owned(String::from_utf8_lossy(hdr.value).to_string()),
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
                        url: match path.parse() {
                            Ok(v) => v,
                            Err(e) => return Err((e, stream)),
                        },
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

const HTTP_SCHEME: &'static str = "http://";
const HTTPS_SCHEME: &'static str = "https://";

#[cfg(test)]
mod test {
    use super::*;
    use anyhow::Context;

    #[test]
    fn test_url() {
        struct TestCase {
            input: &'static str,
            expect: Option<&'static str>,
            expect_error: bool,
        }

        let cases = vec![
            TestCase {
                input: "google.com",
                expect: None,
                expect_error: true,
            },
            TestCase {
                input: "google.com:443",
                expect: Some("google.com:443"),
                expect_error: false,
            },
            TestCase {
                input: "https://google.com/testpath",
                expect: Some("https://google.com:443/testpath"),
                expect_error: false,
            },
            TestCase {
                input: "http://google.com",
                expect: Some("http://google.com:80"),
                expect_error: false,
            },
            TestCase {
                input: "ftp://google.com",
                expect: None,
                expect_error: true,
            },
        ];

        for TestCase {
            input,
            expect,
            expect_error,
        } in cases
        {
            let actual = HttpUrl::from_str(input)
                .map(|v| v.to_string())
                .with_context(|| format!("Parsing input: {input}"));
            if expect_error {
                assert!(actual.is_err());
            } else {
                assert_eq!(actual.unwrap(), expect.unwrap())
            }
        }
    }
}
