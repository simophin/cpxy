use super::{Protocol, ProtocolAcceptor};
use crate::addr::Address;
use crate::http::parse_response;
use crate::protocol::direct::Direct;
use crate::protocol::ProxyRequest;
use crate::tls::TlsStream;
use anyhow::Context;
use async_shutdown::Shutdown;
use std::net::SocketAddr;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::spawn;

pub async fn test_protocol_valid_config<P>(
    new_protocol: impl FnOnce(SocketAddr) -> P,
    acceptor: Option<impl ProtocolAcceptor + Clone + Send + Sync + 'static>,
) -> anyhow::Result<()>
where
    P: Protocol + Clone + Send + Sync + 'static,
{
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("Bind listener")?;

    let p = new_protocol(listener.local_addr()?);

    let shutdown = Shutdown::new();
    if let Some(acceptor) = acceptor {
        spawn(super::server::run_server(
            shutdown.clone(),
            "test",
            listener,
            acceptor,
            Direct::default(),
        ));
    }

    let conn = p
        .new_stream(
            &ProxyRequest::from("www.baidu.com:443".parse::<Address>().unwrap()),
            &Default::default(),
            None,
        )
        .await?;

    let mut conn = TlsStream::connect_tls("www.baidu.com", conn)
        .await
        .context("Connecting to TLS")?;

    conn.write_all(b"GET / HTTP/1.1\r\nHost: www.baidu.com\r\n\r\n")
        .await
        .context("Writing request")?;

    let mut conn = BufReader::new(conn);

    let status = parse_response(&mut conn, |res| res.code.context("No status code")).await?;

    assert_eq!(status, 200);
    Ok(())
}
