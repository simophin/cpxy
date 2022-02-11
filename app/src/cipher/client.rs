use std::{borrow::Cow, fmt::Display, str::FromStr};

use super::strategy::EncryptionStrategy;
use super::stream::CipherStream;
use crate::{
    http::{parse_response, HttpResponse},
    utils::RWBuffer,
};
use anyhow::{anyhow, Context};
use base64::URL_SAFE_NO_PAD;
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};

fn check_server_response(res: &HttpResponse) -> anyhow::Result<()> {
    match res.status_code {
        101 => Ok(()),
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

pub async fn connect<T: AsyncRead + AsyncWrite + Send + Sync + Unpin>(
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
        stream
            .write_all(initial_data.as_slice())
            .await
            .context("Write initial data")?;
        initial_data.clear();
    }

    // Parse and check response
    initial_data.resize(2048, 0); // Reuse this vector

    let res = parse_response(stream, RWBuffer::new(initial_data))
        .await
        .context("Parsing cipher response")?;

    check_server_response(&res)?;

    let rd_cipher = recv_strategy.wrap_cipher(
        super::suite::create_cipher(cipher_type, key.as_slice(), iv.as_slice())
            .expect("To have created a same cipher as wr_cipher"),
    );

    Ok(CipherStream::new(
        "client".to_string(),
        res,
        rd_cipher,
        wr_cipher,
    ))
}
