use std::{borrow::Cow, net::IpAddr, net::SocketAddr};

use anyhow::{bail, Context};
use orion::aead;
use tokio::net::UdpSocket;

use super::{Request, Response};

pub struct Settings {
    pub addr: SocketAddr,
    pub key: aead::SecretKey,
}

pub async fn fetch(
    settings: &Settings,
    domain: impl AsRef<str> + Send,
) -> anyhow::Result<Vec<IpAddr>> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("Binding udp socket")?;
    socket
        .connect(&settings.addr)
        .await
        .context("connecting to remote")?;

    let req = Request::Recursive(Cow::Borrowed(domain.as_ref()));
    let req = rmp_serde::to_vec(&req).context("serializing")?;
    let req = aead::seal(&settings.key, &req).context("encrypting")?;

    socket.send(&req).await.context("sending")?;

    // Waiting for response...
    let mut recv = req;
    recv.resize(4097, 0);

    let len = socket.recv(&mut recv).await.context("Receiving response")?;
    recv.truncate(len);

    // Decrypting...
    let decrypted = aead::open(&settings.key, &recv).context("Decrypting")?;

    // Deserialize response
    let response: Response<'static> = rmp_serde::from_slice(&decrypted).context("Deserialising")?;

    match response {
        Response::RecursiveResult(result) => Ok(result.into_owned()),
        Response::Err(e) => bail!("Error from server: {e}"),
    }
}
