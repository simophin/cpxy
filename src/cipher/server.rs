use super::client::URL_PREFIX;
use super::CipherStream;
use crate::utils::RWBuffer;
use anyhow::anyhow;
use chacha20::cipher::{NewCipher, StreamCipher};
use chacha20::{ChaCha20, Key, Nonce};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn check_request(req: httparse::Request) -> Result<ChaCha20, (&'static str, &'static str)> {
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
            if base64::decode_config_slice(v, base64::URL_SAFE_NO_PAD, &mut key[..])
                == Ok(key.len())
            {
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
            if base64::decode_config_slice(h.value, base64::URL_SAFE_NO_PAD, &mut nonce[..])
                == Ok(nonce.len())
            {
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
    mut buf: RWBuffer,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
    // Receive and check http request
    let (mut cipher, offset) = loop {
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

        match stream.read(buf.write_buf()).await? {
            0 => return Err(anyhow!("Unexpected EOF")),
            v => buf.advance_write(v),
        }
    };

    buf.advance_read(offset);

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
    use tokio::io::{split, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpSocket;
    use tokio::spawn;

    #[tokio::test]
    async fn server_client_works() {
        let server = TcpSocket::new_v4().unwrap();
        server.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let server_addr = server.local_addr().unwrap();
        let server = spawn(async move { server.listen(1).unwrap().accept().await.unwrap().0 });

        let client = TcpSocket::new_v4().unwrap();
        let client = client.connect(server_addr).await.unwrap();

        let server = server.await.unwrap();
        let server = spawn(async move { listen(server, Default::default()).await });

        let mut input_buf = RWBuffer::with_capacity(8192);
        input_buf.write_all(b"hello,").unwrap();

        let (client_r, mut client_w) =
            split(super::super::client::send(client, input_buf).await.unwrap());
        let (server_r, server_w) = split(server.await.unwrap().unwrap());

        let mut client_r = BufReader::new(client_r);
        let mut server_r = BufReader::new(server_r);

        client_w.write_all(b"world\n").await.unwrap();
        let mut line = Default::default();
        server_r.read_line(&mut line).await.unwrap();
        assert_eq!(line, "hello,world");
    }
}
