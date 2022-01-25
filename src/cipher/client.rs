use super::strategy::EncryptionStrategy;
use super::stream::CipherStream;
use crate::utils::RWBuffer;
use anyhow::anyhow;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn check_server_response(res: httparse::Response<'_, '_>) -> anyhow::Result<()> {
    match res.code {
        Some(101) => Ok(()),
        _ => Err(anyhow!("Expecting http code = 101, got {res:?}")),
    }
}

pub async fn connect<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
    host: &str,
    send_strategy: EncryptionStrategy,
    receive_strategy: EncryptionStrategy,
    // This buffer can contain initial data to send along with connection header
    mut buf: RWBuffer,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
    let (cipher_type, mut wr_cipher, key, iv) = super::suite::pick_cipher();
    wr_cipher = send_strategy.wrap_cipher(wr_cipher);

    stream
        .write_all(
            format!(
                "GET /shop/by-id/{}/{}/{send_strategy}/{receive_strategy}/{cipher_type} HTTP/1.1\r\n\
                      Connection: Upgrade\r\n\
                      Host: {host}\r\n\
                      Upgrade: websocket\r\n\
                      Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                      Sec-WebSocket-Version: 13\r\n\r\n",
                base64::encode_config(key.as_slice(), base64::URL_SAFE_NO_PAD),
                base64::encode_config(iv.as_slice(), base64::URL_SAFE_NO_PAD),
            )
            .as_bytes(),
        )
        .await?;

    // Send initial data encrypted
    if buf.remaining_read() > 0 {
        wr_cipher.apply_keystream(buf.read_buf_mut());
        stream.write_all(buf.read_buf()).await?;
        buf.consume_read();
    }

    // Parse and check response
    loop {
        match stream.read(buf.write_buf()).await? {
            0 => return Err(anyhow!("Unexpected EOF")),
            v => buf.advance_write(v),
        };

        let mut headers = [httparse::EMPTY_HEADER; 20];
        let mut res = httparse::Response::new(&mut headers);
        match res.parse(buf.read_buf())? {
            httparse::Status::Complete(offset) => {
                if let Err(e) = check_server_response(res) {
                    debug_http_error(stream, buf).await;
                    return Err(e);
                }
                buf.advance_read(offset);
                break;
            }
            httparse::Status::Partial => {}
        }
    }

    let mut rd_cipher = receive_strategy.wrap_cipher(
        super::suite::create_cipher(cipher_type, key.as_slice(), iv.as_slice())
            .expect("To have created a same cipher as wr_cipher"),
    );

    // Decrypt the stream before handing over to CipherStream
    if buf.remaining_read() > 0 {
        rd_cipher.apply_keystream(buf.read_buf_mut());
    }

    Ok(CipherStream::new(
        "client".to_string(),
        stream,
        rd_cipher,
        wr_cipher,
        Some(buf),
    ))
}

async fn debug_http_error<T: AsyncRead + AsyncWrite + Unpin>(mut stream: T, buf: RWBuffer) {
    let mut data = Vec::new();
    data.extend_from_slice(buf.read_buf());
    drop(buf);
    let _ = stream.read_to_end(&mut data).await;
    log::error!(
        "Error response as: {}",
        String::from_utf8_lossy(data.as_slice())
    );
}