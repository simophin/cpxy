use super::{Protocol, ProtocolAcceptor};
use crate::addr::Address;
use crate::http::parse_response;
use crate::http::writer::RequestWriter;
use crate::protocol::direct::Direct;
use crate::protocol::{NoopProtocolReporter, ProtocolReporter, ProxyRequest};
use crate::tls::TlsStream;
use anyhow::Context;
use async_shutdown::Shutdown;
use hyper::Method;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::BufReader;
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

    let reporter: Arc<dyn ProtocolReporter> = Arc::new(NoopProtocolReporter::default());

    let conn = p
        .new_stream(
            &ProxyRequest::from("www.baidu.com:443".parse::<Address>().unwrap()),
            &reporter,
            None,
        )
        .await?;

    let mut conn = TlsStream::connect_tls("www.baidu.com", conn)
        .await
        .context("Connecting to TLS")?;

    let mut writer = RequestWriter::write(Method::GET, "/");
    writer.write_header("Host", "www.baidu.com");
    writer
        .to_async(&mut conn)
        .await
        .context("Writing request")?;

    let mut conn = BufReader::new(conn);

    let status = parse_response(&mut conn, |res| res.code.context("No status code")).await?;

    assert_eq!(status, 200);
    Ok(())
}
