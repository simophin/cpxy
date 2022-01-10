use async_std::net::{TcpListener, TcpStream};
use async_std::task::spawn;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::cmp::min;
use tide::log;

async fn copy_chucked_to_raw(
    mut r: impl AsyncRead + Sized + Unpin,
    mut w: impl AsyncWrite + Sized + Unpin,
) -> anyhow::Result<()> {
    let mut len_line_buf = [0; 256];
    let mut buf = [0; 40960];
    let buf_size = buf.len();
    while let Ok(len) = r.read(&mut len_line_buf).await {
        if len == 0 {
            break;
        }

        let first_new_line = len_line_buf
            .iter()
            .position(|x| *x == b'\r')
            .ok_or_else(|| anyhow::anyhow!("Invalid chucked"))?;

        if first_new_line >= len - 1 || len_line_buf[first_new_line + 1] != b'\n' {
            return Err(anyhow::anyhow!("Invalid checked length"));
        }

        let mut chuck_size: usize = String::from_utf8_lossy(&len_line_buf[0..first_new_line])
            .as_ref()
            .parse()?;
        chuck_size -= len - first_new_line - 1;
        w.write_all(&len_line_buf[first_new_line + 2..len]).await?;

        while chuck_size > 0 {
            let bytes_read = r.read(&mut buf[0..min(chuck_size, buf_size)]).await?;
            w.write_all(&buf[0..bytes_read]).await?;
            chuck_size -= bytes_read;
        }
    }

    Ok(())
}

async fn copy_raw_to_chucked(
    mut r: impl AsyncRead + Sized + Unpin,
    mut w: impl AsyncWrite + Sized + Unpin,
) -> anyhow::Result<()> {
    use std::io::Write;
    let mut buf = [0; 40960];
    let mut len_line = Vec::new();
    while let Ok(len) = r.read(&mut buf[0..]).await {
        len_line.clear();
        write!(&mut len_line, "{}\r\n", len)?;
        w.write_all(&len_line).await?;
        w.write_all(&buf[0..len]).await?;
    }

    Ok(())
}

fn serve_sock_conn(sock_stream: TcpStream, hostname: String, port: u16) {
    spawn(async move {
        let address = format!("{}:{}", hostname, port);
        log::info!("Connecting to upstream: https://{}", address);
        let (mut upstream_rx, mut upstream_tx) =
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
        let mut s = String::default();
        upstream_rx.read_to_string(&mut s).await?;
        log::info!("Received from upstream: {}", s);

        let (socks_rx, socks_tx) = sock_stream.split();

        let job = spawn(async move { copy_chucked_to_raw(upstream_rx, socks_tx).await });

        copy_raw_to_chucked(socks_rx, upstream_tx).await?;
        job.await
    });
}

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    log::start();

    let hostname = std::env::var("UPSTREAM_HOST").unwrap();
    let port: u16 = std::env::var("UPSTRAEM_PORT")
        .unwrap_or("443".to_string())
        .parse()?;

    let listener = TcpListener::bind("127.0.0.1:8081").await?;

    while let Ok((socks_stream, addr)) = listener.accept().await {
        log::info!("Accepted SOCKS conn from {}", addr);
        serve_sock_conn(socks_stream, hostname.clone(), port);
    }

    Ok(())
}
