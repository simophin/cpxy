use std::sync::Arc;

use super::{Protocol, ProtocolAcceptedState, ProtocolAcceptor};
use crate::io::read_data_with_timeout;
use crate::protocol::{NoopProtocolReporter, ProtocolReporter};
use anyhow::Context;
use async_shutdown::Shutdown;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;

pub async fn run_server(
    shutdown: Shutdown,
    name: &'static str,
    tcp_listener: TcpListener,
    acceptor: impl ProtocolAcceptor + Clone + Send + Sync + 'static,
    upstream: impl Protocol + Clone + Send + Sync + 'static,
) -> anyhow::Result<()> {
    while let Some(r) = shutdown.wrap_cancel(tcp_listener.accept()).await {
        let (conn, addr) = r.context("Accepting connection")?;

        log::info!("{name}: New connection from {addr}");

        let shutdown = shutdown.clone();
        let upstream = upstream.clone();
        let acceptor = acceptor.clone();

        spawn(async move {
            if let Some(Err(e)) = shutdown
                .wrap_cancel(serve_conn(name, conn, acceptor, upstream))
                .await
            {
                log::error!("{name}: Connection error: {e:?}");
            }

            log::info!("{name}: Connection closed from {addr}");
        });
    }

    Ok(())
}

async fn serve_conn(
    name: &'static str,
    conn: TcpStream,
    acceptor: impl ProtocolAcceptor,
    upstream: impl Protocol,
) -> anyhow::Result<()> {
    let (state, req) = acceptor
        .accept(conn)
        .await
        .context("Accepting connection")?;

    log::debug!("{name} got {req:?}");

    let reporter: Arc<dyn ProtocolReporter> = Arc::new(NoopProtocolReporter::default());

    match upstream
        .new_stream(&req, &reporter, None)
        .await
        .context("Connecting to upstream")
    {
        Ok(mut upstream) => {
            let initial_data = read_data_with_timeout(&mut upstream)
                .await
                .context("Read initial data")?;
            let mut conn = state
                .reply_success(initial_data)
                .await
                .context("Replying success")?;

            let _ = copy_bidirectional(&mut conn, &mut upstream).await;
            Ok(())
        }

        Err(err) => {
            state.reply_error(Some(err.to_string())).await?;
            Err(err)
        }
    }
}
