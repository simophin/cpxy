use std::sync::Arc;

use crate::{
    buf::RWBuffer,
    http::{parse_request, HttpRequestBuilder},
    protocol::Protocol,
    socks5::Address,
    url::HttpUrl,
    utils::copy_duplex,
};
use anyhow::Context;
use async_net::TcpListener;
use futures::AsyncWriteExt;
use smol::spawn;

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

            let (reply_http_response, dst, initial_data) = match stream.method.as_ref() {
                "CONNECT" => (true, Address::try_from(stream.path.as_ref())?, None),
                m => {
                    let url: HttpUrl = stream
                        .path
                        .as_ref()
                        .try_into()
                        .with_context(|| format!("Parsing {} as HTTP URL", stream.path))?;

                    let mut builder = HttpRequestBuilder::new(m, url.path.as_ref())?;
                    for (n, v) in &stream.headers {
                        builder.put_header(n.as_ref(), v.as_ref())?;
                    }
                    (false, url.address, Some(builder.finalise()))
                }
            };
            log::debug!("Sending HTTPPROXY://{dst} from {from} to upstream");

            let upstream = match upstream
                .new_stream(
                    &dst,
                    initial_data.as_ref().map(Vec::as_slice),
                    &Default::default(),
                    None,
                )
                .await
            {
                Ok(stream) => stream,
                Err(e) => {
                    log::error!("Error requesting upstream: {e:?}");
                    stream.write_all(b"HTTP/1.1 500 Error\r\n\r\n").await?;
                    return Err(e);
                }
            };

            if reply_http_response {
                stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await?;
            }

            copy_duplex(stream, upstream, None, None).await
        })
        .detach();
    }
}
