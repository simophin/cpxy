use async_std::io::BufReader;
use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use cjk_proxy::chunked::{copy_chunked_to_raw, copy_raw_to_chunked};
use futures::{select, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, FutureExt};
use httparse::Status;
use tide::log;

async fn serve_client(socket: TcpStream, upstream: String) -> anyhow::Result<()> {
    let (rx, mut tx) = socket.split();
    let mut rx = BufReader::with_capacity(40960, rx);

    let consumed = loop {
        let buf = rx.fill_buf().await?;
        let mut headers = [httparse::EMPTY_HEADER; 20];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf) {
            Ok(Status::Complete(offset)) => {
                log::debug!("Received request: {:?}", req);
                break offset;
            }
            Ok(Status::Partial) => continue,
            Err(e) => return Err(anyhow::anyhow!("Error parsing request: {:?}", e)),
        }
    };

    rx.consume_unpin(consumed);

    // Connect to upstream
    let (upstream_rx, mut upstream_tx) = match TcpStream::connect(upstream).await {
        Ok(stream) => stream.split(),
        Err(e) => {
            tx.write_all(b"HTTP/1.1 500 Upstream Error\r\n\r\n").await?;
            return Err(e.into());
        }
    };

    tx.write_all(
        b"HTTP/1.1 101 Switching Protocols\r\n\
        Upgrade: websocket\r\n\
        Connection: Upgrade\r\n\
        Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\
        \r\n",
    )
    .await?;

    let mut upstream_rx = BufReader::with_capacity(40960, upstream_rx);
    select! {
        result = copy_chunked_to_raw(&mut rx, &mut upstream_tx).fuse() => result,
        result = copy_raw_to_chunked(&mut upstream_rx, &mut tx).fuse() => result,
    }
}

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    log::start();

    let bind_address = format!(
        "{}:{}",
        std::env::var("BIND_ADDRESS").unwrap_or("127.0.0.1".into()),
        std::env::var("BIND_PORT").unwrap_or("8383".into())
    );
    log::info!("Start listening on {}", bind_address);
    let listener = TcpListener::bind(bind_address).await?;

    let upstream_address = format!(
        "{}:{}",
        std::env::var("UPSTREAM_HOST").expect("UPSTREAM_HOST to be set"),
        std::env::var("UPSTREAM_PORT").expect("UPSTREAM_PORT to be set"),
    );

    loop {
        let (socket, addr) = listener.accept().await?;
        log::info!("Received request from {}", addr);

        let upstream_address = upstream_address.clone();
        spawn(async move {
            if let Err(e) = serve_client(socket, upstream_address.clone()).await {
                log::error!("Error serving client {}: {}", addr, e);
            }
            log::info!("Dropping {}", addr);
        });
    }
}
