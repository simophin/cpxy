use super::{Protocol, ProtocolAcceptedState, ProtocolAcceptor};
use crate::io::read_data_with_timeout;
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
    loop {
        let (conn, addr) = tcp_listener
            .accept()
            .await
            .context("Accepting connection")?;

        log::info!("{name}: New connection from {addr}");

        let shutdown = shutdown.clone();
        let upstream = upstream.clone();
        let acceptor = acceptor.clone();

        spawn(async move {
            if let Some(Err(e)) = shutdown
                .wrap_cancel(serve_conn(conn, acceptor, upstream))
                .await
            {
                log::error!("{name}: Connection error: {e:?}");
            }

            log::info!("{name}: Connection closed from {addr}");
        });
    }
}

async fn serve_conn(
    conn: TcpStream,
    acceptor: impl ProtocolAcceptor,
    upstream: impl Protocol,
) -> anyhow::Result<()> {
    let (state, req) = acceptor
        .accept(conn)
        .await
        .context("Accepting connection")?;

    match upstream
        .new_stream(&req, &Default::default(), None)
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
            copy_bidirectional(&mut conn, &mut upstream)
                .await
                .context("Copying data")?;
            Ok(())
        }

        Err(err) => {
            state.reply_error(Some(err.to_string())).await?;
            Err(err)
        }
    }
}
