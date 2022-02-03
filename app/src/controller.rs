use crate::client::{run_client, ClientStatistics};
use crate::config::ClientConfig;
use crate::io::TcpListener;
use crate::socks5::Address;
use crate::utils::RWBuffer;
use anyhow::anyhow;
use async_broadcast::Sender;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use serde::Serialize;
use smol::fs::File;
use smol::spawn;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Controller {
    current: (Arc<ClientConfig>, Arc<ClientStatistics>),
    broadcaster: Sender<(Arc<ClientConfig>, Arc<ClientStatistics>)>,
    config_file: PathBuf,
}

#[derive(Default, Serialize, Debug)]
struct ConfigWithStats {
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
}

fn to_json(s: impl Serialize) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&s)?)
}

impl Controller {
    fn get_config_state(&self) -> anyhow::Result<ConfigWithStats> {
        let (config, stats) = &self.current;
        Ok(ConfigWithStats {
            config: config.clone(),
            stats: stats.clone(),
        })
    }

    async fn set_config(&mut self, new_config: ClientConfig) -> anyhow::Result<ConfigWithStats> {
        let config_with_stats = self.get_config_state()?;
        if config_with_stats.config.as_ref() == &new_config {
            return Ok(config_with_stats);
        }
        let stats = Arc::new(ClientStatistics::new(&new_config));
        let config = Arc::new(new_config);

        self.current = (config.clone(), stats.clone());

        let _ = self.broadcaster.broadcast((config.clone(), stats)).await;

        let _ = File::create(&self.config_file)
            .await?
            .write_all(serde_json::to_vec(config.as_ref())?.as_slice())
            .await?;
        log::info!("Config written successfully to {:?}", self.config_file);
        self.get_config_state()
    }

    async fn handle_client(
        &mut self,
        mut sock: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    ) -> anyhow::Result<()> {
        let mut buf = RWBuffer::with_capacity(65536);
        let result: anyhow::Result<Vec<u8>> = loop {
            match sock.read(buf.write_buf()).await? {
                0 => return Err(anyhow!("Unexpected EOF")),
                v => buf.advance_write(v),
            };

            let mut headers = [httparse::EMPTY_HEADER; 40];
            let mut req = httparse::Request::new(&mut headers);
            match req.parse(buf.read_buf())? {
                httparse::Status::Complete(v) => match (req.method, req.path) {
                    (Some(m), Some(p))
                        if m.eq_ignore_ascii_case("get") && p.eq_ignore_ascii_case("/config") =>
                    {
                        break self.get_config_state().and_then(to_json);
                    }
                    (Some(m), Some(p))
                        if m.eq_ignore_ascii_case("post") && p.eq_ignore_ascii_case("/config") =>
                    {
                        let content_length: Option<usize> = req
                            .headers
                            .iter()
                            .find(|h| h.name.eq_ignore_ascii_case("content-type"))
                            .and_then(|h| String::from_utf8_lossy(h.value).as_ref().parse().ok());

                        if content_length.is_none() {
                            break Err(anyhow!("Content length not specified"));
                        }
                        let content_length = content_length.unwrap();

                        drop(req);
                        drop(headers);
                        buf.advance_read(v);

                        buf.compact();
                        if buf.capacity() < content_length {
                            break Err(anyhow!(
                                "Content-Length: {content_length} exceeds buffer length"
                            ));
                        }

                        let buf_to_read = content_length - buf.read_buf().len();

                        let req: ClientConfig =
                            match sock.read_exact(&mut buf.write_buf()[..buf_to_read]).await {
                                Ok(_) => {
                                    buf.advance_write(buf_to_read);
                                    match serde_json::from_slice(buf.read_buf()) {
                                        Ok(v) => v,
                                        Err(e) => break Err(e.into()),
                                    }
                                }
                                Err(e) => break Err(e.into()),
                            };

                        break self.set_config(req).await.and_then(to_json);
                    }
                    _ => break Err(anyhow!("Invalid http request")),
                },
                httparse::Status::Partial => continue,
            };
        };

        buf.clear();

        match result {
            Ok(body) => {
                sock.write_all(
                    format!(
                        "HTTP/1.1 200\r\n\
                    Content-Type: application/json\r\n\
                    Content-Length: {}\r\n\
                    \r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await?;
                sock.write_all(buf.read_buf()).await?;
                Ok(())
            }
            Err(e) => {
                let err_msg = e.to_string();
                sock.write_all(
                    format!(
                        "HTTP/1.1 500\r\n\
                    Content-Type: plain/text\r\n\
                    Content-Length: {}\r\n\
                    \r\n",
                        err_msg.as_bytes().len()
                    )
                    .as_bytes(),
                )
                .await?;
                sock.write_all(err_msg.as_bytes()).await?;
                Ok(())
            }
        }
    }
}

pub async fn run_controller(bind_address: SocketAddr, config_file: &Path) -> anyhow::Result<()> {
    let listener = TcpListener::bind(&Address::IP(bind_address.clone())).await?;

    let config: ClientConfig = if config_file.exists() {
        serde_json::from_reader(std::fs::File::open(config_file)?)?
    } else {
        Default::default()
    };

    log::info!("Started controller on {bind_address}. Config: {config:?}");
    let stats = ClientStatistics::new(&config);

    let (broadcaster, rx) = async_broadcast::broadcast(1);

    let mut controller = Controller {
        current: (Arc::new(config), Arc::new(stats)),
        broadcaster,
        config_file: config_file.to_path_buf(),
    };

    let _client_task = spawn(async move { run_client(rx).await });

    loop {
        let (socket, addr) = listener.accept().await?;
        log::info!("Serving controller client: {addr}");
        match controller.handle_client(socket).await {
            Ok(_) => {}
            Err(e) => {
                log::error!("Error serving client {addr}: {e}");
            }
        }
        log::info!("Client {addr} disconnected");
    }
}
