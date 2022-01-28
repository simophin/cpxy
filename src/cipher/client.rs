use std::{borrow::Cow, fmt::Display, str::FromStr};

use super::strategy::EncryptionStrategy;
use super::stream::CipherStream;
use crate::utils::RWBuffer;
use anyhow::{anyhow, Context};
use base64::URL_SAFE_NO_PAD;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn check_server_response(res: httparse::Response<'_, '_>) -> anyhow::Result<()> {
    match res.code {
        Some(101) => Ok(()),
        _ => Err(anyhow!("Expecting http code = 101, got {res:?}")),
    }
}

pub struct CipherParams<'a> {
    pub key: Cow<'a, [u8]>,
    pub iv: Cow<'a, [u8]>,
    pub send_strategy: EncryptionStrategy,
    pub recv_strategy: EncryptionStrategy,
    pub cipher_type: u8,
}

impl<'a> Display for CipherParams<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "/{}/{}/{}/{}/{}",
            base64::encode_config(&self.key.as_ref(), URL_SAFE_NO_PAD),
            base64::encode_config(&self.iv.as_ref(), URL_SAFE_NO_PAD),
            self.send_strategy,
            self.recv_strategy,
            self.cipher_type
        ))
    }
}

impl<'a> FromStr for CipherParams<'a>
where
    Self: 'a,
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut ss = s.split('/');
        let _ = ss.next();
        match (ss.next(), ss.next(), ss.next(), ss.next(), ss.next()) {
            (Some(k), Some(iv), Some(ss), Some(rs), Some(t)) => Ok(Self {
                key: Cow::Owned(base64::decode_config(k, URL_SAFE_NO_PAD).context("Decoding key")?),
                iv: Cow::Owned(base64::decode_config(iv, URL_SAFE_NO_PAD).context("Decoding iv")?),
                send_strategy: ss.parse().context("parse send_strategy")?,
                recv_strategy: rs.parse().context("parse recv_strategy")?,
                cipher_type: t.parse().context("parse cipher_type")?,
            }),
            _ => return Err(anyhow!("Invalid URL {s}")),
        }
    }
}

pub async fn connect<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
    host: &str,
    send_strategy: EncryptionStrategy,
    recv_strategy: EncryptionStrategy,
    mut initial_data: Vec<u8>,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin> {
    let (cipher_type, mut wr_cipher, key, iv) = super::suite::pick_cipher();
    wr_cipher = send_strategy.wrap_cipher(wr_cipher);

    let params = CipherParams {
        key: Cow::Borrowed(key.as_slice()),
        iv: Cow::Borrowed(iv.as_slice()),
        send_strategy,
        recv_strategy,
        cipher_type,
    };

    stream
        .write_all(
            format!(
                "GET {params} HTTP/1.1\r\n\
                      Connection: Upgrade\r\n\
                      Host: {host}\r\n\
                      Upgrade: websocket\r\n\
                      Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                      Sec-WebSocket-Version: 13\r\n\r\n"
            )
            .as_bytes(),
        )
        .await?;

    // Send initial data encrypted
    if initial_data.len() > 0 {
        wr_cipher.apply_keystream(initial_data.as_mut_slice());
        stream.write_all(initial_data.as_slice()).await?;
        initial_data.clear();
    }

    // Parse and check response
    initial_data.resize(1024, 0); // Reuse this vector
    let mut buf = RWBuffer::new(initial_data);
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

    let mut rd_cipher = recv_strategy.wrap_cipher(
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
