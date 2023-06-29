use anyhow::Context;
use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine};
use bytes::BytesMut;

use crate::cipher::{stream::CipherState, StreamCipher};

pub fn encrypt_initial_data<C: StreamCipher>(
    state: &mut CipherState<C>,
    data: impl AsRef<[u8]>,
) -> String {
    let mut encrypted = BytesMut::from(data.as_ref());
    state.apply(&mut encrypted);
    B64.encode(&encrypted)
}

pub fn decrypt_initial_data<C: StreamCipher>(
    state: &mut CipherState<C>,
    data: impl AsRef<[u8]>,
) -> anyhow::Result<Vec<u8>> {
    let mut decrypted = B64.decode(data.as_ref()).context("decoding base64")?;
    state.apply(&mut decrypted);
    Ok(decrypted)
}
