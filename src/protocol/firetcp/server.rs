use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context;
use async_net::TcpListener;
use chacha20::ChaCha20;
use cipher::KeyIvInit;
use futures::io::copy;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite};
use parking_lot::Mutex;
use smol::{spawn, Task};
use std::net::SocketAddr;

use crate::io::connect_tcp;
use crate::utils::{race, read_bincode_lengthed_async};

use super::cipher::CipherRead;
use super::proto::{CipherOption, Request, INITIAL_CIPHER_LEN, INITIAL_KEY, INITIAL_NONCE};

pub fn run_server(listener: TcpListener) -> Task<anyhow::Result<()>> {
    spawn(async move {
        let clients: Arc<Mutex<BTreeMap<SocketAddr, Task<()>>>> = Default::default();
        loop {
            let (client, addr) = listener.accept().await?;
            log::info!("Accepted client from {addr}");
            let clients_ref = Arc::downgrade(&clients);
            clients.lock().insert(
                addr,
                spawn(async move {
                    if let Err(e) = serve(client).await {
                        log::error!("Error serving {addr}: {e:?}");
                    }
                    if let Some(clients) = clients_ref.upgrade() {
                        clients.lock().remove(&addr);
                    }
                }),
            );
        }
    })
}

pub async fn serve(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let (r, mut w) = stream.split();

    let mut r = CipherRead::<_, _, ChaCha20>::new(
        r,
        INITIAL_CIPHER_LEN,
        ChaCha20::new_from_slices(INITIAL_KEY, INITIAL_NONCE).unwrap(),
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
        .split();

    let upload_task = spawn(async move { copy(r, &mut upstream_w).await });
    let download_task = spawn(async move { copy(upstream_r, &mut w).await });

    race(upload_task, download_task).await?;
    Ok(())
}
