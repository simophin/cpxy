use std::sync::Arc;

use anyhow::Context;
use async_shutdown::Shutdown;
use chacha20::ChaCha20;
use cipher::KeyIvInit;
use futures::io::copy;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::io::connect_tcp;
use crate::utils::{race, read_bincode_lengthed_async};

use super::cipher::CipherRead;
use super::proto::{CipherOption, Request, INITIAL_CIPHER_LEN};
use super::pw::PasswordedKey;

pub async fn run_server(
    shutdown: Shutdown,
    listener: TcpListener,
    password: PasswordedKey,
) -> anyhow::Result<()> {
    let password = Arc::new(password);
    loop {
        let (client, addr) = listener.accept().await?;
        log::info!("Accepted client from {addr}");
        let password = password.clone();

        let shutdown = shutdown.clone();
        spawn(async move {
            if let Err(e) = serve(shutdown, client.compat(), &password).await {
                log::error!("Error serving {addr}: {e:?}");
            }
        });
    }
}

pub async fn serve(
    shutdown: Shutdown,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    password: &PasswordedKey,
) -> anyhow::Result<()> {
    let (r, mut w) = stream.split();

    let mut r = CipherRead::<_, _, ChaCha20>::new(
        r,
        INITIAL_CIPHER_LEN,
        ChaCha20::new_from_slices(password.init_key(), password.init_nonce()).unwrap(),
    );

    let Request {
        est_cipher, addr, ..
    } = read_bincode_lengthed_async(&mut r)
        .await
        .context("Reading request")?;

    r.set_establish_cipher(
        est_cipher.map(|CipherOption { key, nonce }| ChaCha20::new(&key.into(), &nonce.into())),
    )
    .context("Unable to change est cipher")?;

    let (upstream_r, mut upstream_w) = connect_tcp(&addr)
        .await
        .with_context(|| format!("Connecting to upstream {addr}"))?
        .compat()
        .split();

    let upload_task = {
        let shutdown = shutdown.clone();
        spawn(async move { shutdown.wrap_cancel(copy(r, &mut upstream_w)).await })
    };

    let download_task = spawn(async move { shutdown.wrap_cancel(copy(upstream_r, &mut w)).await });

    race(upload_task, download_task).await?;
    Ok(())
}
