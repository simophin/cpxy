use async_std::io::BufReader;
use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use cjk_proxy::chunked::{copy_chunked_to_raw, copy_raw_to_chunked};
use cjk_proxy::cursor::Cursor;
use cjk_proxy::socks5::serve_socks5;
use futures::{select, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, FutureExt};
use httparse::Status;
use tide::log;

async fn serve_client(socket: TcpStream, upstream: String) -> anyhow::Result<()> {
    let (mut rx, mut tx) = socket.split();

    // Parsing requests
    let mut cursor: Cursor<2048> = Default::default();

    loop {
        let bytes_read = rx.read(cursor.remaining_mut()).await?;
        if bytes_read == 0 {
            return Err(anyhow::anyhow!("Unexpected EOF"));
        }
        cursor.move_position(bytes_read);

        let mut headers = [httparse::EMPTY_HEADER; 20];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(cursor.used()) {
            Ok(Status::Complete(_)) => {
                log::debug!("Received request: {:?}", req);
                break;
            }
            Ok(Status::Partial) => continue,
            Err(e) => return Err(anyhow::anyhow!("Error parsing request: {:?}", e)),
        }
    }

    tx.write_all(
        b"HTTP/1.1 101 Switching Protocols\r\n\
        Upgrade: websocket\r\n\
        Connection: Upgrade\r\n\
        Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\
        \r\n",
    )
    .await?;

    serve_socks5(&mut rx, &mut tx).await
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
