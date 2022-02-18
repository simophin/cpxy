use super::client::CipherParams;
use anyhow::bail;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::buf::RWBuffer;
use crate::http::{parse_request, HttpRequest};
use crate::stream::VecStream;

use super::stream::CipherStream;
use super::suite::{create_cipher, BoxedStreamCipher};

fn check_request(
    req: &HttpRequest,
) -> Result<(BoxedStreamCipher, BoxedStreamCipher), (&'static str, &'static str)> {
    // log::debug!("Received request: {:?}", req);
    if !req.method.eq_ignore_ascii_case("get") {
        return Err((
            "HTTP/1.1 401 Unsupported method\r\n\r\n",
            "Unsupported HTTP method",
        ));
    }

    let CipherParams {
        key,
        iv,
        send_strategy: client_send_strategy,
        recv_strategy: client_receive_strategy,
        cipher_type,
    } = match req.path.parse() {
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

pub async fn listen<T: AsyncRead + AsyncWrite + Send + Sync + Unpin>(
    stream: T,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Send + Sync + Unpin> {
    let mut req = match parse_request(stream, RWBuffer::new(512, 65536)).await {
        Ok(v) => v,
        Err((e, mut stream)) => {
            stream.write_all(b"HTTP/1.1 400 invalid request").await?;
            return Err(e.into());
        }
    };

    log::debug!("Got request: {req:?}");

    let (rd_cipher, wr_cipher) = match check_request(&req) {
        Ok(v) => v,
        Err((res, err)) => {
            req.write_all(res.as_bytes()).await?;
            bail!("{err}");
        }
    };

    let initial_data = match req.get_header(super::client::INITIAL_DATA_HEADER) {
        Some(value) => {
            base64::decode_config(value.as_str().as_ref(), super::client::INITIAL_DATA_CONFIG)?
        }
        None => Default::default(),
    };

    // Respond client with correct details
    req.write_all(
        b"HTTP/1.1 101 Switching Protocols\r\n\
                    Upgrade: WebSocket\r\n\
                    Connection: Upgrade\r\n\
                    Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\
                    \r\n",
    )
    .await?;

    let (r, w) = split(req);

    Ok(CipherStream::new(
        "server".to_string(),
        VecStream::new(initial_data).chain(r),
        w,
        rd_cipher,
        wr_cipher,
    ))
}

#[cfg(test)]
mod test {
    use super::super::client::connect;
    use super::super::strategy::EncryptionStrategy;
    use super::*;
    use crate::test::duplex;
    use futures_lite::AsyncReadExt;
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
            let (client_r, client_w) = split(client);
            let mut client = connect(
                client_r,
                client_w,
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
