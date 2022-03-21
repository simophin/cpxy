use std::{borrow::Cow, fmt::Display, str::FromStr};

use super::strategy::EncryptionStrategy;
use super::stream::CipherStream;
use crate::{http::HeaderValue, ws::negotiate_websocket};
use anyhow::{anyhow, Context};
use base64::{display::Base64Display, URL_SAFE_NO_PAD};
use futures_lite::{io::split, AsyncRead, AsyncWrite};
use std::fmt::Write;

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

pub const INITIAL_DATA_CONFIG: base64::Config = base64::URL_SAFE_NO_PAD;
pub const INITIAL_DATA_HEADER: &'static str = "X-Cache-Key";

pub async fn connect(
    mut url: String,
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

    if initial_data.len() > 0 {
        wr_cipher.apply_keystream(&mut initial_data);
    }

    let _ = write!(&mut url, "{}", params);

    let (r, w) = split(
        negotiate_websocket(
            &url,
            vec![(
                Cow::Borrowed(INITIAL_DATA_HEADER),
                HeaderValue::from_display(Base64Display::with_config(
                    &initial_data,
                    INITIAL_DATA_CONFIG,
                )),
            )],
        )
        .await?,
    );

    let rd_cipher = recv_strategy.wrap_cipher(
        super::suite::create_cipher(cipher_type, key.as_slice(), iv.as_slice())
            .expect("To have created a same cipher as wr_cipher"),
    );

    Ok(CipherStream::new(
        "client".to_string(),
        r,
        w,
        rd_cipher,
        wr_cipher,
    ))
}
