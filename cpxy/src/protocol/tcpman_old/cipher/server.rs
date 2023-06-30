use std::fmt::Debug;

use super::client::{CipherParams, BASE64_ENGINE};
use anyhow::{bail, Context};
use base64::decode_engine;
use cipher::StreamCipher;
use tokio::io::{split, AsyncRead, AsyncWrite};

use crate::http::WithHeaders;
use crate::ws::{serve_websocket, WebSocketServeResult};

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

pub struct Handshaker<T, RC, WC> {
    r: WebSocketServeResult<T>,
    rc: RC,
    wc: WC,
}

impl<T, RC, WC> Handshaker<T, RC, WC>
where
    T: AsyncRead + AsyncWrite + Send + Sync + Unpin,
    RC: StreamCipherExt + Send + Sync,
    WC: StreamCipherExt + Send + Sync,
{
    pub async fn respond_error(self, err: &impl Debug) -> anyhow::Result<()> {
        let error_content = format!("{err:?}");
        self.r
            .respond_fail_with_raw_response(
                format!(
                    "HTTP/1.1 500 Internal Error\r\n\
        Content-Type: text/plain\r\n\
        Content-Length: {}\r\n\
        \r\n\
        {error_content}",
                    error_content.as_bytes().len()
                )
                .as_bytes(),
            )
            .await
    }

    pub async fn respond_success(
        self,
    ) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync> {
        let Handshaker { r, rc, wc } = self;

        let (r, w) = split(
            r.respond_success()
                .await
                .context("Responding success to cipher client")?,
        );
        Ok(CipherStream::new("server".to_string(), r, w, rc, wc))
    }
}

pub async fn accept_client<T: AsyncRead + AsyncWrite + Send + Sync + Unpin>(
    stream: T,
) -> anyhow::Result<(
    Option<Vec<u8>>,
    Handshaker<T, impl StreamCipherExt + Send + Sync, impl StreamCipherExt + Send + Sync>,
)> {
    let req = serve_websocket(stream).await?;
    let (mut rd_cipher, wr_cipher) = match check_request(req.request().path.as_ref()) {
        Ok(v) => v,
        Err((res, err)) => {
            req.respond_fail_with_raw_response(res.as_bytes()).await?;
            bail!("{err}");
        }
    };

    let initial_data = match req.request().get_header(super::client::INITIAL_DATA_HEADER) {
        Some(value) => {
            let mut data = decode_engine(value, BASE64_ENGINE)?;
            rd_cipher.apply_keystream(&mut data);
            Some(data)
        }
        None => None,
    };

    Ok((
        initial_data,
        Handshaker {
            r: req,
            rc: rd_cipher,
            wc: wr_cipher,
        },
    ))
}

#[cfg(test)]
mod test {
    use super::super::client::connect;
    use super::super::strategy::EncryptionStrategy;
    use super::*;
    use crate::{
        fetch::connect_http_stream, io::connect_tcp, test::create_http_server, url::HttpUrl,
    };
    use rand::RngCore;
    use tokio::io::{copy, split, AsyncWriteExt};
    use tokio::spawn;

    #[tokio::test]
    async fn test_cipher_server() {
        let (http_server, url) = create_http_server().await;
        let server_task = spawn(async move {
            loop {
                let (stream, _) = http_server.accept().await.unwrap();
                let (initial_data, hs) = accept_client(stream).await.unwrap();
                let (mut r, mut w) = split(hs.respond_success().await.unwrap());
                w.write_all(&initial_data.unwrap_or_default())
                    .await
                    .unwrap();
                copy(&mut r, &mut w).await.unwrap();
            }
        });

        let data = b"hello, world";
        let url = HttpUrl::try_from(url.as_str()).unwrap();
        let stream = connect_http_stream(
            url.is_https,
            &url.address,
            connect_tcp(&url.address).await.unwrap(),
        )
        .await
        .unwrap();

        let mut client = connect(
            &url,
            stream,
            EncryptionStrategy::FirstN(5.try_into().unwrap()),
            EncryptionStrategy::Always,
            Option::<&str>::None,
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
        server_task.abort();
    }
}
