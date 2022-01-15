use futures::{AsyncWrite, AsyncWriteExt};
use httparse::Status;

use crate::parse::ParseError;
use crate::socks5::Address;

#[derive(Debug, Eq, PartialEq)]
pub struct Request {
    pub protocol: String,
    pub address: String,
}

impl Request {
    pub async fn write(
        protocol: &str,
        address: &Address,
        upstream_address: &str,
        tx: &mut (impl AsyncWrite + Unpin + ?Sized),
    ) -> anyhow::Result<()> {
        let url = format!("{}://{}", protocol, address);
        let encoded_url = urlencoding::encode(&url);
        tx.write_all(
            format!(
                "GET /{} HTTP/1.1\r\n\
                Host: {}\r\n\
                Upgrade: websocket\r\n\
                Connection: upgrade\r\n\
                Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                Sec-WebSocket-Version: 13\r\n\r\n",
                encoded_url, upstream_address
            )
            .as_bytes(),
        )
        .await?;
        Ok(())
    }

    pub fn parse(buf: &[u8]) -> Result<Option<Request>, ParseError> {
        let mut headers = [httparse::EMPTY_HEADER; 20];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf) {
            Ok(Status::Complete(_)) => match req.path {
                Some(v) => {
                    let v = urlencoding::decode(&v[1..]).map_err(|_| {
                        ParseError::unexpected("path", v.to_string(), "valid url encoded path")
                    })?;
                    let mut splits = v.split("://");
                    let protocol = splits
                        .next()
                        .ok_or_else(|| ParseError::unexpected("protocol", v.to_string(), "://"))?;
                    let address = splits
                        .next()
                        .ok_or_else(|| ParseError::unexpected("protocol", v.to_string(), "://"))?;
                    if protocol.trim().is_empty() || address.trim().is_empty() {
                        return Err(ParseError::unexpected("protocol/address", "", "non empty"));
                    }

                    Ok(Some(Self {
                        protocol: protocol.to_string(),
                        address: address.to_string(),
                    }))
                }
                _ => return Err(ParseError::unexpected("path", "", "non empty")),
            },
            Ok(Status::Partial) => Ok(None),
            Err(_) => Err(ParseError::unexpected("http req", "", "valid request")),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[async_std::test]
    async fn encoding_works() {
        let mut buf: Vec<u8> = Vec::new();
        Request::write(
            "udp",
            &Address::Name("host".to_string(), 123),
            "localhost:456",
            &mut buf,
        )
        .await
        .unwrap();

        let req = Request::parse(buf.as_ref()).unwrap().unwrap();
        assert_eq!(
            req,
            Request {
                address: "host:123".to_string(),
                protocol: "udp".to_string(),
            }
        );
    }
}
