use bytes::BytesMut;
use hyper::{header, Method};
use std::fmt::Display;
use std::fmt::Write;
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub struct RequestWriter;

impl RequestWriter {
    pub fn write(method: Method, path: impl Display) -> HeaderWriter {
        let mut buf = BytesMut::new();
        write!(buf, "{method} {path} HTTP/1.1\r\n").unwrap();
        HeaderWriter(buf)
    }
}

pub struct ResponseWriter;

impl ResponseWriter {
    pub fn write(code: u16, reason: impl Display) -> HeaderWriter {
        let mut buf = BytesMut::new();
        write!(buf, "HTTP/1.1 {} {}\r\n", code, reason).unwrap();
        HeaderWriter(buf)
    }
}

pub struct HeaderWriter(BytesMut);

impl HeaderWriter {
    pub fn write_header(&mut self, name: impl Display, value: impl Display) -> &mut Self {
        write!(self.0, "{name}: {value}\r\n").unwrap();
        self
    }

    pub fn finish_with_body(
        mut self,
        content_type: impl Display,
        body: impl AsRef<[u8]>,
    ) -> BytesMut {
        self.write_header(header::CONTENT_TYPE, content_type)
            .write_header(header::CONTENT_LENGTH, body.as_ref().len());

        let Self(mut buf) = self;

        buf.extend_from_slice(b"\r\n");
        buf.extend_from_slice(body.as_ref());

        log::debug!(
            "Finished building http parts: {:<20?}",
            std::str::from_utf8(&buf)
        );

        buf
    }

    pub fn finish(mut self) -> BytesMut {
        self.0.extend_from_slice(b"\r\n");
        log::debug!(
            "Finished building http parts: {:<20?}",
            std::str::from_utf8(&self.0)
        );
        self.0
    }

    pub async fn to_async(self, w: &mut (impl AsyncWrite + Unpin)) -> std::io::Result<()> {
        w.write_all(&self.finish()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::parse_request;
    use crate::http::utils::WithHeaders;

    #[tokio::test]
    async fn request_writer_works() {
        let _ = env_logger::try_init();

        let mut buf = vec![0u8; 0];
        let mut writer = RequestWriter::write(Method::GET, "/path");
        writer.write_header("Host", "example.com");
        writer.to_async(&mut buf).await.unwrap();

        let mut buf = buf.as_slice();
        let _ = parse_request(&mut buf, |req| {
            assert_eq!(req.method, Some("GET"));
            assert_eq!(req.path, Some("/path"));
            assert_eq!(req.get_header_text("Host"), Some("example.com"));
            Ok(())
        })
        .await
        .expect("To parse request");
    }
}
