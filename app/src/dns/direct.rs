use std::{net::SocketAddr, time::Duration};

use crate::{buf::Buf, shared_udp::SharedClient};
use anyhow::bail;

use lazy_static::lazy_static;
use smol_timeout::TimeoutExt;

fn parse_id(mut buf: &[u8], _: Option<&SocketAddr>) -> anyhow::Result<u16> {
    use bytes::Buf;
    if buf.len() < 2 {
        bail!("Invalid buf");
    }

    Ok(buf.get_u16())
}

fn shared_client() -> &'static SharedClient<u16> {
    lazy_static! {
        static ref INSTANCE: SharedClient<u16> = SharedClient::new(parse_id).unwrap();
    }
    &INSTANCE
}

pub async fn query_direct(input: Buf, upstream_dns: &SocketAddr) -> anyhow::Result<Buf> {
    let id = parse_id(&input, None)?;
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

#[cfg(test)]
mod test {
    use smol::block_on;

    use crate::dns::req::{Message, TYPE_A};

    use super::*;

    #[test]
    fn test_query_direct() {
        let _ = env_logger::try_init();
        block_on(async move {
            let mut buf = Buf::new_for_udp();
            buf.set_len(0);

            Message::new_resolve_request(&mut buf, 1, "www.google.com").unwrap();

            let res_buf = query_direct(buf, &"8.8.8.8:53".parse().unwrap())
                .await
                .unwrap();
            let msg = Message::parse(&res_buf).unwrap();
            let answer = msg.answers.iter().next().unwrap();
            assert_eq!(answer.t, TYPE_A);
        });
    }
}
