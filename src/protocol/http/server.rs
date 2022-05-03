use std::{borrow::Cow, sync::Arc};

use crate::{
    buf::RWBuffer,
    http::{parse_request, HttpRequest},
    protocol::Protocol,
    proxy::protocol::ProxyRequest,
    rt::{net::TcpListener, spawn},
    url::HttpUrl,
    utils::copy_duplex,
};
use anyhow::Context;
use futures::AsyncWriteExt;

pub async fn serve(
    stream: TcpListener,
    upstream: impl Protocol + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let upstream = Arc::new(upstream);
    loop {
        let (stream, from) = stream.accept().await?;
        log::debug!("Client {from} connected");
        let upstream = upstream.clone();
        spawn(async move {
            let mut stream =
                match parse_request(stream, RWBuffer::new_vec_uninitialised(4096)).await {
                    Ok(v) => v,
                    Err((e, _)) => {
                        return Err(e).with_context(|| format!("Error parsing request for {from}"));
                    }
                };

            let req = match stream.method.as_ref() {
                "CONNECT" => ProxyRequest::TCP {
                    dst: stream
                        .path
                        .parse()
                        .with_context(|| format!("Parsing {} as TCP address", stream.path))?,
                },
                m => {
                    let url: HttpUrl = stream
                        .path
                        .as_ref()
                        .try_into()
                        .with_context(|| format!("Parsing {} as HTTP URL", stream.path))?;

                    ProxyRequest::HTTP {
                        dst: url.address.clone(),
                        https: url.is_https,
                        req: HttpRequest {
                            method: Cow::Borrowed(m),
                            headers: Default::default(),
                            path: url.path.to_owned(),
                        },
                    }
                }
            };
            log::debug!("Sending {req:?} from {from} to upstream");

            let upstream = match upstream
                .new_stream_conn(&req, &Default::default(), None)
                .await
            {
                Ok((stream, _)) => stream,
                Err(e) => {
                    log::error!("Error requesting upstream: {e:?}");
                    stream.write_all(b"HTTP/1.1 500 Error\r\n\r\n").await?;
                    return Err(e);
                }
            };

            if let ProxyRequest::TCP { .. } = &req {
                stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await?;
            }

            copy_duplex(stream, upstream, None, None).await
        })
        .detach();
    }
}
