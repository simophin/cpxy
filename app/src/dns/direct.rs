use std::{net::SocketAddr, time::Duration};

use crate::{
    buf::Buf,
    dns::req::Message,
    shared_udp::{IDParser, SharedClient},
};
use anyhow::bail;
use lazy_static::lazy_static;
use smol_timeout::TimeoutExt;

fn shared_client() -> &'static SharedClient<u16> {
    lazy_static! {
        static ref INSTANCE: SharedClient<u16> = SharedClient::new::<Message>().unwrap();
    }
    &INSTANCE
}

pub async fn query_direct(input: Buf, upstream_dns: &SocketAddr) -> anyhow::Result<Buf> {
    let id = Message::parse_id(&input, None)?;
    let (socket, rx) = shared_client().register(id)?;
    socket.send_to(&input, upstream_dns).await?;
    match rx.recv().timeout(Duration::from_secs(5)).await {
        Some(Ok((buf, _))) => Ok(buf),
        Some(Err(e)) => Err(e.into()),
        None => {
            shared_client().unregister(&id);
            bail!("Timeout")
        }
    }
}
