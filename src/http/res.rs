use crate::parse::ParseError;
use futures::{AsyncWrite, AsyncWriteExt};
use httparse::{Status, EMPTY_HEADER};
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

#[derive(Debug, Eq, PartialEq)]
pub struct Response {
    pub bound_address: SocketAddr,
}

impl Response {
    pub async fn write(
        bound: &SocketAddr,
        w: &mut (impl AsyncWrite + Unpin + ?Sized),
    ) -> anyhow::Result<()> {
        w.write_all(
            format!(
                "HTTP/1.1 101 Switching Protocols\r\n\
                    Upgrade: websocket\r\n\
                    Connection: Upgrade\r\n\
                    X-Bound-Address: {}\r\n\
                    X-Bound-Port: {}\r\n\
                    Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\
                    \r\n",
                bound.ip().to_string(),
                bound.port().to_string(),
            )
            .as_bytes(),
        )
        .await?;
        Ok(())
    }

    pub fn parse(buf: &[u8]) -> Result<Option<Self>, ParseError> {
        let mut headers = [EMPTY_HEADER; 20];
        let mut response = httparse::Response::new(&mut headers);
        match response.parse(buf) {
            Ok(Status::Complete(_)) => {
                if response.code != Some(101) {
                    return Err(ParseError::unexpected("http code", response.code, "101"));
                }

                let bound_address = String::from_utf8_lossy(
                    response
                        .headers
                        .iter()
                        .find(|x| x.name.eq_ignore_ascii_case("X-Bound-Address"))
                        .ok_or_else(|| ParseError::unexpected("bound_address", "", "non empty"))?
                        .value,
                );

                let bound_port: u16 = String::from_utf8_lossy(
                    response
                        .headers
                        .iter()
                        .find(|x| x.name.eq_ignore_ascii_case("X-Bound-Port"))
                        .ok_or_else(|| ParseError::unexpected("bound_port", "", "non empty"))?
                        .value,
                )
                .parse()
                .map_err(|_| ParseError::unexpected("bound port", "", "numeric port"))?;

                let addr = match IpAddr::from_str(bound_address.as_ref()) {
                    Ok(addr) => SocketAddr::new(addr, bound_port),
                    Err(_) => {
                        return Err(ParseError::unexpected(
                            "IP",
                            bound_address.to_string(),
                            "valid IP address",
                        ))
                    }
                };
                Ok(Some(Self {
                    bound_address: addr,
                }))
            }
            Ok(Status::Partial) => Ok(None),
            Err(_) => Err(ParseError::unexpected(
                "http response",
                "",
                "valid response",
            )),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[async_std::test]
    async fn encoding_works() {
        let mut buf: Vec<u8> = Default::default();
        let sock_addr = SocketAddr::from_str("1.2.3.4:80").unwrap();
        Response::write(&sock_addr, &mut buf).await.unwrap();

        assert_eq!(
            Response {
                bound_address: sock_addr
            },
            Response::parse(buf.as_slice()).unwrap().unwrap(),
        );
    }
}
