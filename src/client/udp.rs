use std::{borrow::Cow, time::Duration};

use crate::{protocol::Protocol, utils::new_vec_uninitialised};
use anyhow::{anyhow, Context};
use futures::{select, AsyncRead, AsyncReadExt, AsyncWrite, FutureExt, SinkExt, StreamExt};

use crate::{
    config::ClientConfig, handshake::Handshaker, proxy::protocol::ProxyRequest, rt::spawn,
    socks5::UdpRepr as Socks5UdpRepr, udp_relay::new_udp_relay,
};

use super::{ClientStatistics, UpstreamStatistics};

const UDP_IDLING_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn serve_udp_proxy_conn(
    c: &ClientConfig,
    stats: &ClientStatistics,
    is_v4: bool,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let (relay_addr, tx, mut rx) = match new_udp_relay(is_v4).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("Error creating UDP relay: {e:?}");
            handshaker.respond_err(&mut stream).await?;
            return Err(e);
        }
    };

    handshaker.respond_ok(&mut stream, Some(relay_addr)).await?;

    // Wait for first packet to decide where to go
    let pkt = rx.next().await.context("Waiting for first packet")?;
    let addr = pkt.addr().into_owned();

    let req = ProxyRequest::UDP {
        initial_dst: pkt.addr(),
        initial_data: Cow::Borrowed(pkt.payload()),
    };
    let mut upstreams = c.find_best_upstream(&req, stats, &addr);
    let mut last_error = None;

    while let Some((name, upstream)) = upstreams.pop() {
        log::debug!("Trying upstream {name} for {req:?}");
        let (upstream_sink, upstream_stream) = match upstream.protocol.new_dgram_conn(&req).await {
            Ok(v) => v,
            Err(e) => {
                last_error.replace(e);
                continue;
            }
        }
        .split();

        let upload_task = spawn(
            rx.filter_map(|pkt| async move {
                let addr = pkt.addr().resolve().await.ok()?.next()?;
                Some(Ok((pkt.payload_bytes(), addr)))
            })
            .forward(upstream_sink),
        );

        let download_task = spawn(
            upstream_stream
                .map(|(data, addr)| {
                    Socks5UdpRepr {
                        addr: &addr.into(),
                        frag_no: 0,
                        payload: data,
                    }
                    .to_packet()
                })
                .forward(tx.sink_map_err(|e| anyhow::Error::from(e))),
        );

        return select! {
            _ = upload_task.fuse() => Ok(()),
            _ = download_task.fuse() => Ok(()),
            v = drain_socks(&mut stream).fuse() => v,
        };
    }

    Err(last_error.unwrap_or_else(|| anyhow!("No upstreams available for {req:?}")))
}

async fn drain_socks(socks: &mut (impl AsyncRead + Unpin)) -> anyhow::Result<()> {
    let mut buf = new_vec_uninitialised(24);
    while socks.read(&mut buf).await? > 0 {}
    Ok(())
}

#[cfg(test)]
mod tests {}
