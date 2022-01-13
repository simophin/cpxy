use anyhow::anyhow;
use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use cjk_proxy::chunked::copy_with_cursor;
use cjk_proxy::cursor::Cursor;
use cjk_proxy::parse::{Parsable, ParseError, ParseResult};
use cjk_proxy::socks5::{
    Address, ClientConnRequest, ConnStatusCode, CMD_BIND_UDP, CMD_CONNECT_TCP,
};
use futures::{select, AsyncReadExt, AsyncWriteExt, FutureExt};
use httparse::{Status, EMPTY_HEADER};

#[derive(Debug)]
struct HttpParseResult<'a> {
    bound_address: Address<'a>,
}

impl<'a> Parsable<'a> for HttpParseResult<'a> {
    fn parse(buf: &'a [u8]) -> ParseResult<'a, Self> {
        let mut headers = [EMPTY_HEADER; 20];
        let mut response = httparse::Response::new(&mut headers);
        match response.parse(buf)? {
            Status::Complete(offset) => {
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

                let bound_port = String::from_utf8_lossy(
                    response
                        .headers
                        .iter()
                        .find(|x| x.name.eq_ignore_ascii_case("X-Bound-Port"))
                        .ok_or_else(|| ParseError::unexpected("bound_port", "", "non empty"))?
                        .value,
                );

                Ok(Some((
                    &buf[offset..],
                    Self {
                        bound_address: Address::from(
                            bound_address.as_ref(),
                            bound_port.as_ref().parse().map_err(|_| {
                                ParseError::unexpected(
                                    "bound port",
                                    bound_port.to_string(),
                                    "numeric port",
                                )
                            })?,
                        )?,
                    },
                )))
            }
            Status::Partial => Ok(ParseStatus::Incomplete),
        }
    }
}

async fn serve_sock_conn(sock_stream: TcpStream, hostname: &str, port: u16) -> anyhow::Result<()> {
    let (mut socks_rx, mut socks_tx) = sock_stream.split();

    let ClientConnRequest { cmd, address } =
        wait_for_handshake(&mut socks_rx, &mut socks_tx).await?;

    log::info!(
        "ClientConnRequest(cmd = {:?}, address = {:?})",
        cmd,
        address
    );

    let url = match cmd {
        CMD_CONNECT_TCP => urlencoding::encode(&format!("tcp://{}", address)).into_owned(),
        CMD_BIND_UDP => urlencoding::encode(&format!("udp://{}", address)).into_owned(),
        _ => {
            write_response(
                &mut socks_tx,
                ConnStatusCode::UNSUPPORTED_COMMAND,
                &Default::default(),
            )
            .await?;
            return Err(anyhow!("CMD_BIND_TCP unsupported"));
        }
    };

    let upstream_address = format!("{}:{}", hostname, port);
    let (mut upstream_rx, mut upstream_tx) =
        match async_native_tls::connect(hostname, TcpStream::connect(&upstream_address).await?)
            .await
        {
            Ok(r) => r.split(),
            Err(e) => {
                write_response(&mut socks_tx, ConnStatusCode::FAILED, &Default::default()).await?;
                return Err(e.into());
            }
        };
    log::info!("Connected to upstream: {}", upstream_address);

    // Handshake request
    if let Err(e) = upstream_tx
        .write_all(
            format!(
                "GET /{} HTTP/1.1\r\n\
                Host: {}\r\n\
                Upgrade: websocket\r\n\
                Connection: upgrade\r\n\
                Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                Sec-WebSocket-Version: 13\r\n\r\n",
                url, upstream_address
            )
            .as_bytes(),
        )
        .await
    {
        write_response(&mut socks_tx, ConnStatusCode::FAILED, &Default::default()).await?;
        return Err(e.into());
    }

    let mut cursor: Cursor<41960> = Default::default();
    let HttpParseResult { bound_address } = match parse_cursor(&mut cursor, &mut upstream_rx).await
    {
        Ok(r) => r,
        Err(e) => {
            write_response(&mut socks_tx, ConnStatusCode::FAILED, &Default::default()).await?;
            return Err(e.into());
        }
    };

    match cmd {
        CMD_CONNECT_TCP => {
            write_response(&mut socks_tx, ConnStatusCode::GRANTED, &bound_address).await?;

            select! {
                r1 = copy_with_cursor(&mut cursor, &mut upstream_rx, &mut socks_tx).fuse() => r1?,
                r2 = async_std::io::copy(&mut socks_rx, &mut upstream_tx).fuse() => {
                    r2?;
                },
            };

            Ok(())
        }

        CMD_BIND_UDP => Ok(()),

        _ => panic!("Unsupported command"),
    }
}

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let hostname = std::env::var("UPSTREAM_HOST").unwrap();
    let port: u16 = std::env::var("UPSTREAM_PORT")
        .unwrap_or("443".to_string())
        .parse()?;

    let listener = TcpListener::bind("127.0.0.1:8081").await?;

    while let Ok((socks_stream, addr)) = listener.accept().await {
        log::info!("Accepted SOCKS conn from {}", addr);
        let hostname = hostname.clone();
        spawn(async move {
            if let Err(e) = serve_sock_conn(socks_stream, &hostname, port).await {
                log::error!("Error serving {}: {}", addr, e);
            }
            log::info!("Dropping {}", addr);
        });
    }

    Ok(())
}
