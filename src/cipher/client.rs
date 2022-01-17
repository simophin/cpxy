use crate::cipher::stream::CipherStream;
use crate::utils::RWBuffer;
use anyhow::anyhow;
use chacha20::cipher::{NewCipher, StreamCipher};
use chacha20::{ChaCha20, Key, Nonce};
use rand::Rng;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn check_server_response(res: httparse::Response<'_, '_>) -> anyhow::Result<()> {
    match res.code {
        Some(101) => Ok(()),
        v => Err(anyhow!("Expecting http code = 101, got {:?}", v)),
    }
}

pub const URL_PREFIX: &'static str = "/shop/by-id/";

pub async fn send<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
    // This buffer can contain initial data to send along with connection header
    mut buf: RWBuffer,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
    let key: [u8; 32] = rand::thread_rng().gen();
    let key = Key::from(key);
    let nonce: [u8; 12] = rand::thread_rng().gen();
    let nonce = Nonce::from(nonce);

    stream
        .write_all(
            format!(
                "GET {URL_PREFIX}{} HTTP/1.1\r\n\
                      Connection: Upgrade;\r\n\
                      Upgrade: websocket\r\n\
                      Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                      Sec-WebSocket-Version: 13\r\n\
                      X-Cache-Key: {}\r\n\r\n",
                base64::encode_config(key.as_slice(), base64::URL_SAFE_NO_PAD),
                base64::encode_config(nonce.as_slice(), base64::URL_SAFE_NO_PAD),
            )
            .as_bytes(),
        )
        .await?;

    // Send initial data encrypted
    let mut cipher = ChaCha20::new(&key, &nonce);
    if buf.remaining_read() > 0 {
        cipher.apply_keystream(buf.read_buf_mut());
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

    // Decrypt the stream before handing over to CipherStream
    if buf.remaining_read() > 0 {
        cipher.apply_keystream(buf.read_buf_mut());
    }

    Ok(CipherStream::new(4096, stream, cipher, Some(buf)))
}
