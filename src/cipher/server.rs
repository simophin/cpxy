use super::strategy::EncryptionStrategy;
use anyhow::anyhow;
use base64::{decode_config, URL_SAFE_NO_PAD};
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use lazy_static::lazy_static;
use regex::Regex;
use std::str::FromStr;

use crate::utils::RWBuffer;

use super::stream::CipherStream;
use super::suite::{create_cipher, BoxedStreamCipher};

fn check_request(
    req: httparse::Request,
) -> Result<(BoxedStreamCipher, BoxedStreamCipher), (&'static str, &'static str)> {
    // log::debug!("Received request: {:?}", req);
    match req.method {
        Some(m) if m.eq_ignore_ascii_case("get") => {}
        _ => {
            return Err((
                "HTTP/1.1 401 Unsupported method\r\n\r\n",
                "Unsupported HTTP method",
            ))
        }
    };

    lazy_static! {
        static ref RE: Regex = Regex::new(r"/shop/by-id/(.+?)/(.+?)/(.+?)/(.+?)/(\d+?)").unwrap();
    }

    let path = req.path.unwrap_or("");

    let (key, iv, client_send_strategy, client_receive_strategy, t) = RE
        .captures(path)
        .and_then(
            |cap| match (cap.get(1), cap.get(2), cap.get(3), cap.get(4), cap.get(5)) {
                (Some(k), Some(i), Some(s1), Some(s2), Some(t)) => Some((
                    decode_config(k.as_str(), URL_SAFE_NO_PAD).ok()?,
                    decode_config(i.as_str(), URL_SAFE_NO_PAD).ok()?,
                    EncryptionStrategy::from_str(s1.as_str()).ok()?,
                    EncryptionStrategy::from_str(s2.as_str()).ok()?,
                    t.as_str().parse::<u8>().ok()?,
                )),
                _ => None,
            },
        )
        .ok_or_else(|| ("HTTP/1.1 201 OK\r\n\r\n", "Invalid URL"))?;

    let rd_cipher = client_send_strategy.wrap_cipher(
        create_cipher(t, key.as_slice(), iv.as_slice())
            .map_err(|_| ("HTTP/1.1 401 Invalid type\r\n\r\n", "Invalid cipher type"))?,
    );

    let wr_cipher = client_receive_strategy
        .wrap_cipher(create_cipher(t, key.as_slice(), iv.as_slice()).unwrap());

    Ok((rd_cipher, wr_cipher))
}

pub async fn listen<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
    let mut buf = RWBuffer::default();

    // Receive and check http request
    let ((mut rd_cipher, wr_cipher), offset) = loop {
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
        rd_cipher.apply_keystream(buf.read_buf_mut());
    }

    Ok(CipherStream::new(
        "server".to_string(),
        stream,
        rd_cipher,
        wr_cipher,
        Some(buf),
    ))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::cipher::client::connect;
    use crate::test::duplex;
    use rand::RngCore;
    use smol::spawn;

    #[test]
    fn test_cipher_server() {
        smol::block_on(async move {
            let (client, server) = duplex(256).await;
            let server_task = spawn(async move {
                let mut server = listen(server).await.expect("To listen");
                let mut buf = vec![0u8; 256];
                loop {
                    let n = match server
                        .read(buf.as_mut_slice())
                        .await
                        .expect("To read cipher stream")
                    {
                        0 => break,
                        v => v,
                    };

                    server
                        .write_all(&buf.as_slice()[..n])
                        .await
                        .expect("To reply to client");
                }
            });

            let data = b"hello, world";
            let mut client = connect(
                client,
                "localhost",
                EncryptionStrategy::FirstN(5.try_into().unwrap()),
                EncryptionStrategy::Always,
                data.to_vec(),
            )
            .await
            .expect("To connect to server");

            let mut buf = Vec::new();

            buf.resize(data.len(), 0);
            client
                .read_exact(buf.as_mut_slice())
                .await
                .expect("To read server response");
            assert_eq!(buf, data);

            let mut data = vec![0u8; 65536];
            rand::thread_rng().fill_bytes(data.as_mut_slice());
            client
                .write_all(data.as_slice())
                .await
                .expect("To write to server");
            buf.resize(data.len(), 0);
            client
                .read_exact(buf.as_mut_slice())
                .await
                .expect("To read second server response");
            assert_eq!(buf, data);

            drop(client);
            let _ = server_task.cancel();
        });
    }
}
