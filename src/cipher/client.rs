use crate::cipher::stream::CipherStream;
use crate::utils::RWBuffer;
use anyhow::anyhow;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn check_server_response(res: httparse::Response<'_, '_>) -> anyhow::Result<()> {
    match res.code {
        Some(101) => Ok(()),
        v => Err(anyhow!("Expecting http code = 101, got {:?}", v)),
    }
}

pub async fn connect<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
    // This buffer can contain initial data to send along with connection header
    mut buf: RWBuffer,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
    let (cipher_type, mut wr_cipher, key, iv) = super::suite::pick_cipher();

    stream
        .write_all(
            format!(
                "GET /shop/by-id/{}/{}/{cipher_type} HTTP/1.1\r\n\
                      Connection: Upgrade;\r\n\
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
                check_server_response(res)?;
                buf.advance_read(offset);
                break;
            }
            httparse::Status::Partial => {}
        }
    }

    let mut rd_cipher = super::suite::create_cipher(cipher_type, key.as_slice(), iv.as_slice())
        .expect("To have created a same cipher as wr_cipher");

    // Decrypt the stream before handing over to CipherStream
    if buf.remaining_read() > 0 {
        rd_cipher.apply_keystream(buf.read_buf_mut());
    }

    Ok(CipherStream::new(
        "client".to_string(),
        4096,
        stream,
        rd_cipher,
        wr_cipher,
        Some(buf),
    ))
}
