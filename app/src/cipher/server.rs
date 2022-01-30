use super::client::CipherParams;
use anyhow::anyhow;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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

    let CipherParams {
        key,
        iv,
        send_strategy: client_send_strategy,
        recv_strategy: client_receive_strategy,
        cipher_type,
    } = match req.path.unwrap_or("").parse() {
        Ok(v) => v,
        Err(e) => {
            log::error!("Error parsing params: {e}");
            return Err(("HTTP/1.1 404 Not found\r\n\r\n", "Invalid PATH"));
        }
    };

    let rd_cipher = client_send_strategy.wrap_cipher(
        create_cipher(cipher_type, key.as_ref(), iv.as_ref())
            .map_err(|_| ("HTTP/1.1 401 Invalid type\r\n\r\n", "Invalid cipher type"))?,
    );

    let wr_cipher = client_receive_strategy
        .wrap_cipher(create_cipher(cipher_type, key.as_ref(), iv.as_ref()).unwrap());

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
    use super::super::client::connect;
    use super::super::strategy::EncryptionStrategy;
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
