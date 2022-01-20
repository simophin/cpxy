use anyhow::anyhow;
use base64::{decode_config, decode_config_slice, URL_SAFE_NO_PAD};
use cipher::StreamCipher;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::utils::RWBuffer;

use super::stream::CipherStream;
use super::suite::{create_cipher, CipherIv, CipherKey, CipherType};

fn check_request(
    req: httparse::Request,
) -> Result<
    (
        CipherType,
        CipherKey,
        CipherIv,
        Box<dyn StreamCipher + Send + Sync>,
    ),
    (&'static str, &'static str),
> {
    log::debug!("Received request: {:?}", req);
    match req.method {
        Some(m) if m.eq_ignore_ascii_case("get") => {}
        _ => {
            return Err((
                "HTTP/1.1 401 Unsupported method\r\n\r\n",
                "Unsupported HTTP method",
            ))
        }
    };

    let path = req.path.unwrap_or("");
    let (key, iv, t) = sscanf::scanf!(path, "/shop/by-id/{}/{}/{}", String, String, u8)
        .and_then(|(k, i, t)| {
            Some((
                decode_config(k, URL_SAFE_NO_PAD).ok()?,
                decode_config(i, URL_SAFE_NO_PAD).ok()?,
                t,
            ))
        })
        .ok_or_else(|| ("HTTP/1.1 201 OK\r\n\r\n", "Invalid URL"))?;

    let cipher = create_cipher(t, key.as_slice(), iv.as_slice())
        .map_err(|_| ("HTTP/1.1 401 Invalid type\r\n\r\n", "Invalid cipher type"))?;

    Ok((t, key, iv, cipher))
}

pub async fn listen<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
    let mut buf = RWBuffer::default();

    // Receive and check http request
    let ((cipher_type, key, iv, mut rd_cipher), offset) = loop {
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
        4096,
        stream,
        rd_cipher,
        create_cipher(cipher_type, key.as_slice(), iv.as_slice())
            .expect("To create same cipher as read cipher"),
        Some(buf),
    ))
}
