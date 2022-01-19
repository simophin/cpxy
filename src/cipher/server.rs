use super::client::URL_PREFIX;
use super::stream::CipherStream;
use crate::utils::RWBuffer;
use anyhow::anyhow;
use base64::{decode_config_slice, URL_SAFE_NO_PAD};
use chacha20::cipher::{NewCipher, StreamCipher};
use chacha20::{ChaCha20, Key, Nonce};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn check_request(req: httparse::Request) -> Result<ChaCha20, (&'static str, &'static str)> {
    log::debug!("Received request: {:?}", req);
    match req.method {
        Some(m) if m.eq_ignore_ascii_case("get") => {}
        _ => {
            return Err((
                "HTTP/1.1 401 Unsupported method\r\n\r\n",
                "Unsupported HTTP method",
            ))
        }
    };

    let mut key = [0u8; 32];
    let mut nonce = [0u8; 12];

    req.path
        .and_then(|v| {
            if v.starts_with(URL_PREFIX) {
                Some(&v[URL_PREFIX.len()..])
            } else {
                None
            }
        })
        .and_then(|v| {
            if decode_config_slice(v, URL_SAFE_NO_PAD, &mut key[..]) == Ok(key.len()) {
                Some(())
            } else {
                None
            }
        })
        .ok_or(("HTTP/1.1 201 OK\r\n\r\n", "Invalid URL"))?;

    req.headers
        .iter()
        .find(|x| x.name.eq_ignore_ascii_case("X-Cache-Key"))
        .and_then(|h| {
            if decode_config_slice(h.value, URL_SAFE_NO_PAD, &mut nonce[..]) == Ok(nonce.len()) {
                Some(())
            } else {
                None
            }
        })
        .ok_or(("HTTP/1.1 200 OK\r\n\r\n", "Invalid nonce"))?;

    Ok(ChaCha20::new(
        Key::from_slice(&key),
        Nonce::from_slice(&nonce),
    ))
}

pub async fn listen<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
    let mut buf = RWBuffer::default();

    // Receive and check http request
    let (mut cipher, offset) = loop {
        match stream.read(buf.write_buf()).await? {
            0 => return Err(anyhow!("Unexpected EOF")),
            v => buf.advance_write(v),
        }

        let mut headers = [httparse::EMPTY_HEADER; 20];
        let mut req = httparse::Request::new(&mut headers);

        match req.parse(buf.read_buf()) {
            Ok(httparse::Status::Complete(offset)) => {
                break (
                    match check_request(req) {
                        Ok(v) => v,
                        Err((res, msg)) => {
                            stream.write_all(res.as_bytes()).await?;
                            return Err(anyhow!("Error listening: {}", msg));
                        }
                    },
                    offset,
                )
            }
            Err(e) => {
                stream.write_all(b"HTTP/1.1 400 invalid request").await?;
                return Err(e.into());
            }
            _ => {}
        }
    };

    buf.advance_read(offset);

    // Respond client with correct details
    stream
        .write_all(
            b"HTTP/1.1 101 Switching Protocols\r\n\
                    Upgrade: websocket\r\n\
                    Connection: Upgrade\r\n\
                    Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\
                    \r\n",
        )
        .await?;

    // Decrypt the initial data
    if buf.remaining_read() > 0 {
        cipher.apply_keystream(buf.read_buf_mut());
    }

    Ok(CipherStream::new(4096, stream, cipher, Some(buf)))
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;
    use std::time::Duration;
    use tokio::io::{duplex, split, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::spawn;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    #[tokio::test]
    async fn server_client_works() -> anyhow::Result<()> {
        let duration = Duration::from_secs(2000);
        let (client, server) = duplex(4096);

        let server: JoinHandle<anyhow::Result<()>> = spawn(async move {
            let mut server = listen(server).await?;
            let mut buf = RWBuffer::default();
            loop {
                match server.read(buf.write_buf()).await? {
                    0 => return Ok(()),
                    v => buf.advance_write(v),
                };

                server.write_all(buf.read_buf()).await?;
                buf.consume_read();
            }
        });

        let mut init_buf = RWBuffer::default();
        init_buf.write_all(b"hello,")?;

        let (client_r, mut client_w) =
            split(timeout(duration, super::super::client::connect(client, init_buf)).await??);
        let mut client_r = BufReader::new(client_r);
        client_w.write_all(b"world\n").await?;

        let mut line = Default::default();
        timeout(duration, client_r.read_line(&mut line)).await??;

        assert_eq!(line, "hello,world\n");

        drop(client_r);
        drop(client_w);

        server.await?
    }
}
