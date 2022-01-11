use async_std::io::BufReader;
use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use cjk_proxy::chunked;
use futures::{select, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, FutureExt};
use httparse::{Status, EMPTY_HEADER};
use tide::log;

async fn serve_sock_conn(
    sock_stream: TcpStream,
    hostname: String,
    port: u16,
) -> anyhow::Result<()> {
    let address = format!("{}:{}", hostname, port);
    let (upstream_rx, mut upstream_tx) =
        match async_native_tls::connect(&hostname, TcpStream::connect(&address).await?).await {
            Ok(s) => {
                log::info!("Connected to upstream: {}", address);
                s.split()
            }
            Err(e) => {
                log::error!("Error connecting to upstream: {}: {}", address, e);
                return Err(e.into());
            }
        };

    // Handshake request
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        address
    );

    upstream_tx.write_all(req.as_bytes()).await?;

    // Handshake response
    let mut upstream_rx = BufReader::with_capacity(40960, upstream_rx);

    // Parse headers
    let consumed = loop {
        let mut headers = [EMPTY_HEADER; 20];
        let mut response = httparse::Response::new(&mut headers);
        let buf = upstream_rx.fill_buf().await?;
        if buf.len() == 0 {
            return Ok(());
        }

        log::debug!("Received {} bytes from upstream", buf.len());

        match response
            .parse(buf)
            .map_err(|e| anyhow::anyhow!("Error parsing headers: {:?}", e))?
        {
            Status::Complete(offset) => {
                if response.code != Some(101) {
                    return Err(anyhow::anyhow!(
                        "Expecting 101 status code, got: {:?}",
                        response
                    ));
                }

                match response
                    .headers
                    .iter()
                    .find(|x| x.name.eq_ignore_ascii_case("Upgrade"))
                {
                    Some(h) if h.value.eq_ignore_ascii_case(b"websocket") => {
                        log::debug!("Got upstream response: {:?}", response);
                        break offset;
                    }
                    _ => {
                        return Err(anyhow::anyhow!("Expecting transfer encoding to be chunked"));
                    }
                }
            }
            Status::Partial => continue,
        }
    };

    upstream_rx.consume_unpin(consumed);

    let (socks_rx, mut socks_tx) = sock_stream.split();
    let mut socks_rx = BufReader::with_capacity(40960, socks_rx);

    select! {
        result = chunked::copy_chunked_to_raw(
            &mut upstream_rx,
            &mut socks_tx,
        ).fuse() => result,

        result = chunked::copy_raw_to_chunked(
            &mut socks_rx,
            &mut upstream_tx,
        ).fuse() => result,
    }
}

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    log::start();

    let hostname = std::env::var("UPSTREAM_HOST").unwrap();
    let port: u16 = std::env::var("UPSTREAM_PORT")
        .unwrap_or("443".to_string())
        .parse()?;

    let listener = TcpListener::bind("127.0.0.1:8081").await?;

    while let Ok((socks_stream, addr)) = listener.accept().await {
        log::info!("Accepted SOCKS conn from {}", addr);
        let hostname = hostname.clone();
        spawn(async move {
            if let Err(e) = serve_sock_conn(socks_stream, hostname.clone(), port).await {
                log::error!("Error serving {}: {}", addr, e);
            }
            log::info!("Dropping {}", addr);
        });
    }

    Ok(())
}
