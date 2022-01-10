use async_std::io::BufReader;
use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use cjk_proxy::chucked;
use futures::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use httparse::{Status, EMPTY_HEADER};
use tide::log;

fn serve_sock_conn(sock_stream: TcpStream, hostname: String, port: u16) {
    spawn(async move {
        let address = format!("{}:{}", hostname, port);
        log::info!("Connecting to upstream: https://{}", address);
        let (upstream_rx, mut upstream_tx) =
            async_native_tls::connect(&hostname, TcpStream::connect(address).await?)
                .await?
                .split();

        // Handshake request
        let req = format!(
            "\
        GET /\r\n\
        Host: {}\r\n\
        Transfer-Encoding: chucked\r\n\
        Connection: keep-alive\r\n\r\n\
        ",
            hostname
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
                return Err(anyhow::anyhow!("Unexpected EOL"));
            }

            match response
                .parse(buf)
                .map_err(|e| anyhow::anyhow!("Error parsing headers: {:?}", e))?
            {
                Status::Complete(offset) => {
                    if response.code != Some(200) {
                        return Err(anyhow::anyhow!(
                            "Expecting 200 status code, got: {:?}",
                            response.code
                        ));
                    }

                    match response
                        .headers
                        .iter()
                        .position(|x| x.name.eq_ignore_ascii_case("Transfer-Encoding"))
                    {
                        Some(i) if headers[i].value.eq_ignore_ascii_case(b"chucked") => {
                            break offset
                        }
                        _ => {
                            return Err(anyhow::anyhow!(
                                "Expecting transfer encoding to be chucked"
                            ))
                        }
                    }
                }
                Status::Partial => continue,
            }
        };

        upstream_rx.consume_unpin(consumed);

        let (socks_rx, socks_tx) = sock_stream.split();
        let socks_rx = BufReader::with_capacity(40960, socks_rx);

        let job = spawn(async move { chucked::copy_chucked_to_raw(upstream_rx, socks_tx).await });
        chucked::copy_raw_to_chucked(socks_rx, upstream_tx).await?;
        job.await
    });
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
        serve_sock_conn(socks_stream, hostname.clone(), port);
    }

    Ok(())
}
