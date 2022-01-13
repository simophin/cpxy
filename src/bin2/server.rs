use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use futures::{select, AsyncReadExt, AsyncWriteExt, FutureExt};
use httparse::Status;
use std::borrow::Cow;

use cjk_proxy::cursor::Cursor;
use cjk_proxy::parse::{Parsable, ParseError, ParseResult};

#[derive(Debug)]
struct ProxyRequest<'a> {
    protocol: Cow<'a, str>,
    address: Cow<'a, str>,
}

impl<'a> Parsable<'a> for ProxyRequest<'a> {
    fn parse(buf: &'a [u8]) -> ParseResult<'a, Self> {
        let mut headers = [httparse::EMPTY_HEADER; 20];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf)? {
            Status::Complete(offset) => match req.path {
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

                    Ok(Some((
                        &buf[offset..],
                        Self {
                            protocol: Cow::Borrowed(protocol),
                            address: Cow::Borrowed(address),
                        },
                    )))
                }
                _ => return Err(ParseError::unexpected("path", "", "non empty")),
            },
            Status::Partial => Ok(None),
        }
    }
}

async fn serve_client(socket: TcpStream) -> anyhow::Result<()> {
    let (mut rx, mut tx) = socket.split();

    // Parsing requests
    let mut cursor: Cursor<2048> = Default::default();

    let ProxyRequest { protocol, address } = match parse_cursor(&mut cursor, &mut rx).await {
        Ok(r) => r,
        Err(e) => {
            tx.write_all(b"HTTP/1.1 500\r\n\r\n").await?;
            return Err(e);
        }
    };

    match protocol.as_ref() {
        "tcp" => {
            let upstream = match TcpStream::connect(address.as_ref()).await {
                Ok(r) => r,
                Err(e) => {
                    tx.write_all(b"HTTP/1.1 500\r\n\r\n").await?;
                    return Err(e.into());
                }
            };

            let bound = upstream.local_addr().unwrap();
            tx.write_all(
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

            let (upstream_rx, upstream_tx) = upstream.split();

            select! {
                r1 = async_std::io::copy(upstream_rx, tx).fuse() => r1?,
                r2 = async_std::io::copy(rx, upstream_tx).fuse() => r2?,
            };
            Ok(())
        }

        _ => {
            tx.write_all(b"HTTP/1.1 401\r\n\r\n").await?;
            return Ok(());
        }
    }
}

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let bind_address = format!(
        "{}:{}",
        std::env::var("BIND_ADDRESS").unwrap_or("127.0.0.1".into()),
        std::env::var("BIND_PORT").unwrap_or("8383".into())
    );
    log::info!("Start listening on {}", bind_address);
    let listener = TcpListener::bind(bind_address).await?;

    loop {
        let (socket, addr) = listener.accept().await?;
        log::info!("Received request from {}", addr);

        spawn(async move {
            if let Err(e) = serve_client(socket).await {
                log::error!("Error serving client {}: {}", addr, e);
            }
            log::info!("Dropping {}", addr);
        });
    }
}
