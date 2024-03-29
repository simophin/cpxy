use std::{borrow::Cow, fmt::Display, str::FromStr};

use super::strategy::EncryptionStrategy;
use super::stream::CipherStream;
use crate::{http::HttpRequestBuilder, url::HttpUrl, ws::negotiate_websocket};
use anyhow::{anyhow, Context};
use base64::{
    alphabet, decode_engine,
    display::Base64Display,
    engine::fast_portable::{self, FastPortable},
};
use cipher::StreamCipher;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite};

pub const BASE64_ENGINE: &FastPortable =
    &FastPortable::from(&alphabet::URL_SAFE, fast_portable::NO_PAD);

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
            base64::display::Base64Display::from(self.key.as_ref(), BASE64_ENGINE),
            base64::display::Base64Display::from(self.iv.as_ref(), BASE64_ENGINE),
            self.send_strategy,
            self.recv_strategy,
            self.cipher_type
        ))
    }
}

impl FromStr for CipherParams<'static> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut ss = s.split('/');
        let _ = ss.next();
        match (ss.next(), ss.next(), ss.next(), ss.next(), ss.next()) {
            (Some(k), Some(iv), Some(ss), Some(rs), Some(t)) => Ok(Self {
                key: Cow::Owned(decode_engine(k, BASE64_ENGINE).context("Decoding key")?),
                iv: Cow::Owned(decode_engine(iv, BASE64_ENGINE).context("Decoding iv")?),
                send_strategy: ss.parse().context("parse send_strategy")?,
                recv_strategy: rs.parse().context("parse recv_strategy")?,
                cipher_type: t.parse().context("parse cipher_type")?,
            }),
            _ => return Err(anyhow!("Invalid URL {s}")),
        }
    }
}

pub const INITIAL_DATA_HEADER: &'static str = "X-Cache-Key";

pub async fn connect(
    url: &HttpUrl<'_>,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    send_strategy: EncryptionStrategy,
    recv_strategy: EncryptionStrategy,
    auth: Option<impl Display>,
    mut initial_data: impl AsMut<[u8]> + Send,
) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync> {
    let (cipher_type, wr_cipher, key, iv) = super::suite::pick_cipher();
    let mut wr_cipher = send_strategy.wrap_cipher(wr_cipher);

    let params = CipherParams {
        key: Cow::Borrowed(key.as_slice()),
        iv: Cow::Borrowed(iv.as_slice()),
        send_strategy,
        recv_strategy,
        cipher_type,
    };

    if initial_data.as_mut().len() > 0 {
        wr_cipher.apply_keystream(initial_data.as_mut());
    }

    let mut builder = HttpRequestBuilder::new("GET", params)?;
    builder
        .put_header_text("Host", url.address.get_host())?
        .put_header_text(
            INITIAL_DATA_HEADER,
            Base64Display::from(initial_data.as_mut(), BASE64_ENGINE),
        )?;

    if let Some(auth) = auth {
        builder.put_header_text("Authorization", auth)?;
    }

    let (r, w) = negotiate_websocket(builder, stream).await?.split();

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
