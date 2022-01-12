use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use cjk_proxy::cursor::Cursor;
use cjk_proxy::parse::{parse_cursor, Parsable, ParseStatus};
use futures::{select, AsyncReadExt, AsyncWriteExt, FutureExt};
use httparse::Status;

#[derive(Debug)]
struct ProxyRequest {
    protocol: String,
    address: String,
}

impl Parsable for ProxyRequest {
    fn parse(buf: &[u8]) -> anyhow::Result<ParseStatus<(usize, Self)>>
    where
        Self: Sized,
    {
        let mut headers = [httparse::EMPTY_HEADER; 20];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf)? {
            Status::Complete(offset) => match req.path {
                Some(v) => {
                    let v = urlencoding::decode(&v[1..])?;
                    let mut splits = v.split("://");
                    let protocol = splits
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Invalid url: {}", v))?;
                    let address = splits
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Invalid url: {}", v))?;
                    if protocol.trim().is_empty() || address.trim().is_empty() {
                        return Err(anyhow::anyhow!("Invalid url: {}", v));
                    }

                    Ok(ParseStatus::Completed((
                        offset,
                        Self {
                            protocol: protocol.to_string(),
                            address: address.to_string(),
                        },
                    )))
                }
                _ => return Err(anyhow::anyhow!("Unknown path: {:?}", req.path)),
            },
            Status::Partial => Ok(ParseStatus::Incomplete),
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

    match protocol.as_str() {
        "tcp" => {
            let upstream = match TcpStream::connect(address).await {
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
