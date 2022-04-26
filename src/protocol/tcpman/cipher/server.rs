use super::client::CipherParams;
use anyhow::bail;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite};

use crate::stream::VecStream;
use crate::ws::serve_websocket;

use super::stream::CipherStream;
use super::suite::{create_cipher, StreamCipherExt};

fn check_request(
    path: &str,
) -> Result<
    (
        impl StreamCipherExt + Send + Sync + 'static,
        impl StreamCipherExt + Send + Sync + 'static,
    ),
    (&'static str, &'static str),
> {
    let CipherParams {
        key,
        iv,
        send_strategy: client_send_strategy,
        recv_strategy: client_receive_strategy,
        cipher_type,
    } = match path.parse() {
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
    let req = serve_websocket(stream).await?;

    let (rd_cipher, wr_cipher) = match check_request(req.request().path.as_ref()) {
        Ok(v) => v,
        Err((res, err)) => {
            req.respond_fail_with_raw_response(res.as_bytes()).await?;
            bail!("{err}");
        }
    };

    let initial_data = match req.request().get_header(super::client::INITIAL_DATA_HEADER) {
        Some(value) => {
            base64::decode_config(value.as_str().as_ref(), super::client::INITIAL_DATA_CONFIG)?
        }
        None => Default::default(),
    };

    // Respond client with correct details
    let (r, w) = req.respond_success().await?.split();

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
    use crate::{fetch::connect_http, rt, test::create_http_server, url::HttpUrl};
    use futures::{io::copy, AsyncReadExt, AsyncWriteExt};
    use rand::RngCore;

    #[test]
    fn test_cipher_server() {
        rt::block_on(async move {
            let (http_server, url) = create_http_server().await;
            let server_task = rt::spawn(async move {
                loop {
                    let (stream, _) = http_server.accept().await.unwrap();
                    let (r, mut w) = listen(stream).await.unwrap().split();
                    copy(r, &mut w).await.unwrap();
                }
            });

            let data = b"hello, world";
            let url = HttpUrl::try_from(url.as_str()).unwrap();
            let mut client = connect(
                &url,
                connect_http(url.is_https, &url.address).await.unwrap(),
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
