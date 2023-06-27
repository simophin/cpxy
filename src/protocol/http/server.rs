use std::sync::Arc;

use crate::protocol::{ProxyRequest, Stats};
use crate::{http::parse_request, protocol::Protocol, socks5::Address};
use anyhow::{bail, Context};
use async_shutdown::Shutdown;
use bytes::BytesMut;
use hyper::http;
use std::fmt::Write;
use tokio::io::{copy_bidirectional, AsyncBufRead, AsyncWrite, BufReader};
use tokio::net::TcpListener;
use tokio::spawn;

pub async fn serve_http_proxy<P>(
    shutdown: Shutdown,
    stream: TcpListener,
    upstream: P,
) -> anyhow::Result<()>
where
    P: Protocol + Send + Sync + 'static,
{
    let upstream = Arc::new(upstream);
    loop {
        let (stream, from) = stream.accept().await?;
        log::debug!("Client {from} connected");
        let upstream = upstream.clone();
        let shutdown = shutdown.clone();
        spawn(async move {
            if let Some(Err(e)) = shutdown
                .wrap_cancel(serve_http_proxy_conn(
                    BufReader::new(stream),
                    upstream.as_ref(),
                    &Default::default(),
                    None,
                ))
                .await
            {
                log::error!("Error serving http: {e:?}");
            }
        });
    }
}

async fn serve_http_proxy_conn<S, P>(
    mut stream: S,
    upstream_protocol: &P,
    stats: &Stats,
    fwmark: Option<u32>,
) -> anyhow::Result<()>
where
    P: Protocol + Send + Sync + 'static,
    S: AsyncBufRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let req = extract_proxy_request(&mut stream).await?;
    let mut upstream = upstream_protocol
        .new_stream(&req, stats, fwmark)
        .await
        .context("new stream")?;

    copy_bidirectional(&mut stream, &mut upstream)
        .await
        .context("copying data")?;

    Ok(())
}

pub async fn extract_proxy_request<S>(stream: &mut S) -> anyhow::Result<ProxyRequest>
where
    S: AsyncBufRead + Unpin,
{
    parse_request(stream, move |req| {
        match req.method.context("expecting method")? {
            m if m.eq_ignore_ascii_case("CONNECT") => {
                log::debug!("Handshaking http tunnel");
                extract_http_tunnel_request(req)
            }

            _ => {
                log::debug!("Handshaking http proxy");
                extract_http_proxy_request(req)
            }
        }
    })
    .await
}

fn extract_http_tunnel_request(req: &httparse::Request<'_, '_>) -> anyhow::Result<ProxyRequest> {
    Ok(ProxyRequest {
        dst: req
            .path
            .context("expecting uri")?
            .parse()
            .context("parsing uri")?,
        tls: false,
        initial_data: None,
    })
}

fn extract_http_proxy_request(req: &httparse::Request<'_, '_>) -> anyhow::Result<ProxyRequest> {
    let uri: http::uri::Uri = req
        .path
        .context("expecting uri")?
        .parse()
        .context("parsing uri")?;

    let tls = match uri.scheme() {
        Some(&http::uri::Scheme::HTTP) => false,
        Some(&http::uri::Scheme::HTTPS) => true,
        Some(s) => bail!("Unknown http scheme: {s}"),
        _ => bail!("No scheme in uri"),
    };

    let host = uri.host().context("No host in uri")?;
    let dst = match (uri.port(), tls) {
        (None, true) => Address::from((host, 443)),
        (None, false) => Address::from((host, 80)),
        (Some(port), _) => Address::from((host, port.as_u16())),
    };
    let method = req.method.context("expecting method")?;
    let has_host_header = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("host"))
        .is_some();

    // Build a new HTTP request for initial data
    let mut initial_data = BytesMut::new();
    write!(initial_data, "{method} {host} HTTP/1.1\r\n")?;

    // Write a host header if it doesn't exist
    if !has_host_header {
        write!(initial_data, "Host: {host}\r\n")?;
    }

    // Write the rest of http header
    for header in req.headers.iter() {
        write!(initial_data, "{}: ", header.name)?;
        initial_data.extend_from_slice(header.value);
        write!(initial_data, "\r\n")?;
    }
    initial_data.extend_from_slice(b"\r\n");

    Ok(ProxyRequest {
        dst,
        tls,
        initial_data: Some(initial_data.freeze()),
    })
}

pub async fn serve_connect() -> anyhow::Result<()> {
    todo!()
}
