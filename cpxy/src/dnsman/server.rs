use std::{borrow::Cow, net::SocketAddr, sync::Arc};

use super::{DomainListProvider, Request, Response};
use anyhow::Context;
use async_shutdown::Shutdown;
use bytes::BytesMut;
use dnsclient::{r#async::DNSClient, UpstreamServer};
use orion::aead;
use tokio::{net::UdpSocket, spawn};

pub struct Settings {
    pub key: aead::SecretKey,
}

pub async fn run_server<P>(
    shutdown: Shutdown,
    settings: Settings,
    socket: UdpSocket,
    provider: P,
) -> anyhow::Result<()>
where
    P: DomainListProvider + Send + Sync + Clone + 'static,
{
    let socket = Arc::new(socket);

    let mut buf = BytesMut::zeroed(4096);
    let settings = Arc::new(settings);

    loop {
        let (len, src) = socket
            .recv_from(&mut buf)
            .await
            .context("Waiting for packet")?;

        let buf = &buf[..len];
        let req = match aead::open(&settings.key, buf) {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error decrypting request: {e:?}");
                continue;
            }
        };

        let req: super::Request<'static> = match rmp_serde::from_slice(&req) {
            Ok(v) => v,
            Err(e) => {
                log::error!("Error deserializing request: {e:?}");
                continue;
            }
        };

        let socket = socket.clone();
        let settings = settings.clone();
        let shutdown = shutdown.clone();
        let provider = provider.clone();

        spawn(async move {
            if let Some(Err(e)) = shutdown
                .wrap_cancel(serve_conn(socket.as_ref(), &settings, &src, req, provider))
                .await
            {
                log::error!("Error handling connection: {e:?} from {src:?}");
            }
        });
    }
}

async fn serve_conn<P: DomainListProvider>(
    socket: &UdpSocket,
    settings: &Settings,
    src: &SocketAddr,
    req: Request<'static>,
    provider: P,
) -> anyhow::Result<()> {
    log::debug!("Handling {req:?} from {src}");

    let res = match req {
        Request::Recursive(domain) => {
            let domain = domain.as_ref();
            let server = match provider.dns_server_for_domain(domain).await {
                Some(addr) => {
                    log::debug!("Use {addr} to resolve {domain}");
                    Ok(DNSClient::new(vec![UpstreamServer { addr }]))
                }

                None => {
                    log::debug!("Use system resolvers to resolve {domain}");
                    DNSClient::new_with_system_resolvers().context("Creating DNSClient")
                }
            };

            match server {
                Ok(v) => v.query_addrs(domain).await.map_or_else(
                    |err| Response::Err(Cow::Owned(format!("{err:?}"))),
                    |addrs| Response::RecursiveResult(Cow::Owned(addrs)),
                ),

                Err(err) => Response::Err(Cow::Owned(format!("{err:?}"))),
            }
        }
    };

    log::debug!("Responding {src} with {res:?}");

    let res = rmp_serde::to_vec(&res).context("Serialising")?;
    let res = aead::seal(&settings.key, &res).context("Encrypting")?;
    socket
        .send_to(&res, src)
        .await
        .context("Writing response to client")?;

    Ok(())
}
