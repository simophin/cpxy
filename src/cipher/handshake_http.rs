use crate::cipher::stream::CipherStream;
use crate::utils::RWBuffer;
use anyhow::anyhow;
use chacha20::cipher::{NewCipher, StreamCipher};
use chacha20::{ChaCha20, Key, Nonce};
use httparse::{Status, EMPTY_HEADER};
use rand::Rng;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn send<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
    mut buf: RWBuffer,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
    let key: [u8; 32] = rand::thread_rng().gen();
    let key = Key::from(key);
    let nonce: [u8; 12] = rand::thread_rng().gen();
    let nonce = Nonce::from(nonce);

    stream
        .write_all(
            format!(
                "GET /shop/by-id/{}\r\n\
                      Connection: Upgrade;\r\n\
                      Upgrade: websocket\r\n\
                      Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                      Sec-WebSocket-Version: 13\r\n\r\n\
                      X-Cache-Key: {}\r\n\r\n",
                base64::encode(key.as_slice()),
                base64::encode(nonce.as_slice()),
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
            _ => {}
        }

        let mut headers = [EMPTY_HEADER; 20];
        let mut res = httparse::Response::new(&mut headers);
        match res.parse(buf.read_buf())? {
            Status::Complete(offset) => {}
            Status::Partial => {}
        }
    }

    unimplemented!()
}
