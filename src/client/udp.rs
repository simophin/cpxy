use std::{borrow::Cow, time::Duration};

use crate::{
    io::{is_one_off_udp_query, Timer},
    protocol::Protocol,
    utils::new_vec_uninitialised,
};
use anyhow::{anyhow, Context};
use futures::{
    select, AsyncRead, AsyncReadExt, AsyncWrite, FutureExt, SinkExt, StreamExt, TryStreamExt,
};
use smol_timeout::TimeoutExt;

use crate::{
    config::ClientConfig, handshake::Handshaker, proxy::protocol::ProxyRequest, rt::spawn,
    socks5::UdpRepr as Socks5UdpRepr, udp_relay::new_udp_relay,
};

use super::ClientStatistics;

const UDP_IDLING_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn serve_udp_proxy_conn(
    c: &ClientConfig,
    stats: &ClientStatistics,
    is_v4: bool,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    handshaker: Handshaker,
) -> anyhow::Result<()> {
    let (relay_addr, mut tx, mut rx) = match new_udp_relay(is_v4).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("Error creating UDP relay: {e:?}");
            handshaker.respond_err(&mut stream).await?;
            return Err(e);
        }
    };

    handshaker.respond_ok(&mut stream, Some(relay_addr)).await?;

    // Wait for first packet to decide where to go
    let pkt = rx.next().await.context("Waiting for first packet")??;
    let addr = pkt.addr().into_owned();

    let req = ProxyRequest::UDP {
        initial_dst: pkt.addr(),
        initial_data: Cow::Borrowed(pkt.payload()),
    };
    let mut upstreams = c.find_best_upstream(&req, stats, &addr);
    let mut last_error = None;

    while let Some((name, upstream)) = upstreams.pop() {
        log::debug!("Trying upstream {name} for {req:?}");
        let (upstream_sink, mut upstream_stream) = match upstream
            .protocol
            .new_dgram_conn(
                &req,
                &stats.get_protocol_stats(name).unwrap_or_default(),
                c.fwmark,
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                last_error.replace(e.into());
                continue;
            }
        };

        if is_one_off_udp_query(&addr) {
            match upstream_stream.next().timeout(UDP_IDLING_TIMEOUT).await {
                None => {
                    last_error.replace(anyhow!("Timeout waiting for response"));
                    break;
                }
                Some(None) => {
                    last_error.replace(anyhow!("Unexpected EOF while waiting for response"));
                    break;
                }
                Some(Some(Ok((data, addr)))) => {
                    tx.send(
                        Socks5UdpRepr {
                            addr: &addr.into(),
                            payload: data,
                            frag_no: 0,
                        }
                        .to_packet()?,
                    )
                    .await?;
                }
                Some(Some(Err(e))) => {
                    last_error.replace(e);
                    break;
                }
            };

            return Ok(());
        }

        let timer = Timer::new(UDP_IDLING_TIMEOUT);

        let upload_task = {
            let timer = timer.clone();
            spawn(
                rx.inspect(move |_| timer.reset())
                    .map_ok(|pkt| (pkt.payload_bytes(), pkt.addr().into_owned()))
                    .forward(upstream_sink),
            )
        };

        let download_task = {
            let timer = timer.clone();
            spawn(
                upstream_stream
                    .map(|item| {
                        item.and_then(|(data, addr)| {
                            Socks5UdpRepr {
                                addr: &addr.into(),
                                frag_no: 0,
                                payload: data,
                            }
                            .to_packet()
                        })
                    })
                    .inspect(move |_| timer.reset())
                    .forward(tx.sink_map_err(|e| anyhow::Error::from(e))),
            )
        };

        return select! {
            _ = upload_task.fuse() => Ok(()),
            _ = download_task.fuse() => Ok(()),
            _ = timer.fuse() => Ok(()),
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
