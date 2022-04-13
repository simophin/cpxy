use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::rt::{
    mpmc::{bounded, Receiver, Sender},
    spawn, Task, TimeoutExt,
};
use anyhow::{anyhow, Context};
use futures_util::{select, FutureExt};
use serde::{Deserialize, Serialize};

use crate::{
    http::{AsyncHttpStream, HttpRequest},
    rt::net::{TcpListener, TcpStream},
    utils::{copy_duplex, read_bincode_lengthed_async, write_bincode_lengthed_async},
    ws::serve_websocket,
};

use super::{client::ClientCommand, ConnectionId};

#[derive(Serialize, Deserialize)]
pub enum ConnectionType {
    Main,
    Connection { id: ConnectionId },
}

struct Server {
    current_controller_task: Mutex<Option<Task<()>>>,
    client_cmd_tx: Sender<ClientCommand>,
    client_cmd_rx: Receiver<ClientCommand>,
    pending_connections:
        Mutex<HashMap<ConnectionId, Sender<AsyncHttpStream<HttpRequest<'static>, TcpStream>>>>,
}

impl Default for Server {
    fn default() -> Self {
        let (client_cmd_tx, client_cmd_rx) = bounded(10);
        Self {
            current_controller_task: Default::default(),
            client_cmd_rx,
            client_cmd_tx,
            pending_connections: Default::default(),
        }
    }
}

async fn serve_control_connection(
    stream: TcpStream,
    _: SocketAddr,
    server: Arc<Server>,
) -> anyhow::Result<()> {
    let mut req = serve_websocket(stream).await?.respond_success().await?;

    match read_bincode_lengthed_async(&mut req).await? {
        ConnectionType::Main => {
            let client_cmd_rx = server.client_cmd_rx.clone();
            let mut current_task = server
                .current_controller_task
                .lock()
                .map_err(|_| anyhow!("Unable to lock current controller"))?;
            *current_task = Some(spawn(async move {
                log::info!("Control connection established");
                while let Ok(client_cmd) = client_cmd_rx.recv().await {
                    if let Err(e) = write_bincode_lengthed_async(&mut req, &client_cmd).await {
                        log::error!("Error writing cmd {client_cmd:?} to client: {e:?}");
                        break;
                    }
                }
            }));
        }
        ConnectionType::Connection { id } => {
            let pending_sender = server
                .pending_connections
                .lock()
                .map_err(|_| anyhow!("Error locking pending connection"))?
                .remove(&id);
            if let Some(tx) = pending_sender {
                tx.send(req).await?;
            }
        }
    }

    Ok(())
}

struct ConnIdGuard<'a> {
    conn_id: &'a String,
    server: &'a Server,
}

impl<'a> Drop for ConnIdGuard<'a> {
    fn drop(&mut self) {
        if let Ok(mut c) = self.server.pending_connections.lock() {
            let _ = c.remove(self.conn_id);
        }
    }
}

async fn serve_socks_connection(
    stream: TcpStream,
    addr: SocketAddr,
    server: Arc<Server>,
) -> anyhow::Result<()> {
    log::info!("Receive SOCK connection from {addr}");
    let conn_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = bounded(1);
    server
        .pending_connections
        .lock()
        .map_err(|_| anyhow!("Error locking pending connection"))?
        .insert(conn_id.clone(), tx);

    let _conn_guard = ConnIdGuard {
        conn_id: &conn_id,
        server: &server,
    };

    server
        .client_cmd_tx
        .send(ClientCommand::NewConnection {
            id: conn_id.clone(),
        })
        .await?;

    let client_stream = rx
        .recv()
        .timeout(Duration::from_secs(10))
        .await
        .context("Timeout waiting for client connection")?
        .context("Reading client connection channel")?;

    copy_duplex(stream, client_stream, None, None).await
}

pub async fn run_server(
    control_listener: TcpListener,
    socks_listener: TcpListener,
) -> anyhow::Result<()> {
    let server = Arc::new(Server::default());
    loop {
        select! {
            r1 = control_listener.accept().fuse() => {
                let (stream, addr) = r1?;
                let server = server.clone();
                spawn(async move {
                    if let Err(e) = serve_control_connection(stream, addr, server).await {
                        log::error!("Error serving control connection from {addr}: {e:?}");
                    }
                }).detach();
            }

            r2 = socks_listener.accept().fuse() => {
                let (stream, addr) = r2?;
                let server = server.clone();
                spawn(async move {
                    if let Err(e) = serve_socks_connection(stream, addr, server).await {
                        log::error!("Error serving socks5 connection from {addr}: {e:?}");
                    }
                }).detach();
            }
        }
    }
}
