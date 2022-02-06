use crate::broadcast::bounded;
use crate::client::{run_client, ClientStatistics};
use crate::config::ClientConfig;
use crate::io::TcpListener;
use crate::socks5::Address;
use crate::utils::RWBuffer;
use anyhow::anyhow;
use async_broadcast::Sender;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use rust_embed::RustEmbed;
use serde::Serialize;
use smol::fs::File;
use smol::spawn;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Controller {
    current: (Arc<ClientConfig>, Arc<ClientStatistics>),
    broadcaster: Sender<(Arc<ClientConfig>, Arc<ClientStatistics>)>,
    config_file: PathBuf,
}

fn to_json(s: impl Serialize) -> Result<Response, ErrorResponse> {
    Ok(Response::Json(
        serde_json::to_vec(&s).map_err(|e| ErrorResponse::Generic(e.into()))?,
    ))
}

#[derive(RustEmbed)]
#[folder = "$OUT_DIR/webapp"]
struct Asset;

enum Response {
    Empty,
    Json(Vec<u8>),
    EmbedFile {
        path: String,
        file: rust_embed::EmbeddedFile,
    },
}

enum ErrorResponse {
    Generic(anyhow::Error),
    NotFound(Option<String>),
    InvalidRequest,
}

impl Controller {
    fn get_stats(&self) -> Result<Arc<ClientStatistics>, ErrorResponse> {
        Ok(self.current.1.clone())
    }

    fn get_config(&self) -> Result<Arc<ClientConfig>, ErrorResponse> {
        Ok(self.current.0.clone())
    }

    async fn set_config(
        &mut self,
        new_config: ClientConfig,
    ) -> Result<Arc<ClientConfig>, ErrorResponse> {
        match self.get_config()? {
            v if v.as_ref() == &new_config => {
                return Ok(v);
            }
            _ => {},
        }
        let stats = Arc::new(ClientStatistics::new(&new_config));
        let config = Arc::new(new_config);

        self.current = (config.clone(), stats.clone());

        let _ = self.broadcaster.broadcast((config.clone(), stats)).await;

        let _ = File::create(&self.config_file)
            .await
            .map_err(|e| ErrorResponse::Generic(e.into()))?
            .write_all(
                serde_yaml::to_vec(config.as_ref())
                    .map_err(|e| ErrorResponse::Generic(e.into()))?
                    .as_slice(),
            )
            .await
            .map_err(|e| ErrorResponse::Generic(e.into()))?;
        log::info!("Config written successfully to {:?}", self.config_file);
        self.get_config()
    }

    async fn handle_client(
        &mut self,
        mut sock: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    ) -> anyhow::Result<()> {
        let mut buf = RWBuffer::with_capacity(65536);
        let result: Result<Response, ErrorResponse> = loop {
            match sock.read(buf.write_buf()).await? {
                0 => return Err(anyhow!("Unexpected EOF")),
                v => buf.advance_write(v),
            };

            let mut headers = [httparse::EMPTY_HEADER; 40];
            let mut req = httparse::Request::new(&mut headers);
            match req.parse(buf.read_buf())? {
                httparse::Status::Complete(v) => match (req.method, req.path) {
                    (Some(m), _) if m.eq_ignore_ascii_case("option") => {
                        break Ok(Response::Empty);
                    }
                    (Some(m), Some(p))
                        if m.eq_ignore_ascii_case("get")
                            && p.starts_with("/api/config") =>
                    {
                        break self.get_config().and_then(to_json);
                    }
                    (Some(m), Some(p))
                        if m.eq_ignore_ascii_case("get")
                            && p.starts_with("/api/stats") =>
                    {
                        break self.get_stats().and_then(to_json);
                    }
                    (Some(m), Some(p))
                        if m.eq_ignore_ascii_case("post")
                            && p.starts_with("/api/config") =>
                    {
                        let content_length: Option<usize> = req
                            .headers
                            .iter()
                            .find(|h| h.name.eq_ignore_ascii_case("content-type"))
                            .and_then(|h| String::from_utf8_lossy(h.value).as_ref().parse().ok());

                        if content_length.is_none() {
                            break Err(ErrorResponse::InvalidRequest);
                        }
                        let content_length = content_length.unwrap();

                        drop(req);
                        drop(headers);
                        buf.advance_read(v);

                        buf.compact();
                        if buf.capacity() < content_length {
                            break Err(ErrorResponse::InvalidRequest);
                        }

                        let buf_to_read = content_length - buf.read_buf().len();

                        let req: ClientConfig =
                            match sock.read_exact(&mut buf.write_buf()[..buf_to_read]).await {
                                Ok(_) => {
                                    buf.advance_write(buf_to_read);
                                    match serde_json::from_slice(buf.read_buf()) {
                                        Ok(v) => v,
                                        Err(e) => break Err(ErrorResponse::Generic(e.into())),
                                    }
                                }
                                Err(e) => break Err(ErrorResponse::Generic(e.into())),
                            };

                        break self.set_config(req).await.and_then(to_json);
                    }
                    (Some(m), Some(p)) if m.eq_ignore_ascii_case("get") => {
                        let p = match p {
                            "/" => "index.html",
                            v if v.starts_with('/') => &p[1..],
                            v => v,
                        };
                        match Asset::get(p) {
                            Some(file) => {
                                break Ok(Response::EmbedFile {
                                    path: p.to_string(),
                                    file,
                                })
                            }
                            None => break Err(ErrorResponse::NotFound(Some(p.to_string()))),
                        };
                    }
                    _ => break Err(ErrorResponse::NotFound(req.path.map(|v| v.to_string()))),
                },
                httparse::Status::Partial => continue,
            };
        };

        buf.clear();

        match result {
            Ok(Response::Empty) => {
                sock.write_all(b"HTTP/1.1 201\r\nAccess-Control-Allow-Origin: *\r\n\r\n")
                    .await?;
            }
            Ok(Response::Json(buf)) => {
                sock.write_all(
                    format!(
                        "HTTP/1.1 200\r\n\
                    Content-Type: application/json\r\n\
                    Access-Control-Allow-Origin: *\r\n\
                    Content-Length: {}\r\n\
                    \r\n",
                        buf.len()
                    )
                    .as_bytes(),
                )
                .await?;
                sock.write_all(buf.as_slice()).await?;
            }
            Ok(Response::EmbedFile { path, file }) => {
                let mime = mime_guess::from_path(path);
                let mime = match mime.first() {
                    Some(v) => Cow::Owned(v.to_string()),
                    None => Cow::Borrowed("application/octet-stream"),
                };

                sock.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\n\
                Content-Type: {}\r\n\
                Content-Length: {}\r\n\
                \r\n",
                        mime,
                        file.data.as_ref().len()
                    )
                    .as_bytes(),
                )
                .await?;
                sock.write_all(file.data.as_ref()).await?;
            }
            Err(e) => {
                let (code, msg) = match e {
                    ErrorResponse::Generic(e) => (500, e.to_string()),
                    ErrorResponse::InvalidRequest => (400, String::new()),
                    ErrorResponse::NotFound(s) => {
                        (404, format!("Resource {} not found", s.unwrap_or_default()))
                    }
                };
                let msg = msg.as_bytes();
                sock.write_all(
                    format!(
                        "HTTP/1.1 {code}\r\n\
                    Content-Type: text/plain\r\n\
                    Content-Length: {}\r\n\
                    \r\n",
                        msg.len()
                    )
                    .as_bytes(),
                )
                .await?;
                sock.write_all(msg).await?;
            }
        };
        Ok(())
    }
}

pub async fn run_controller(bind_address: SocketAddr, config_file: &Path) -> anyhow::Result<()> {
    let listener = TcpListener::bind(&Address::IP(bind_address.clone())).await?;

    let config = if config_file.exists() {
        Arc::new(serde_yaml::from_reader(std::fs::File::open(config_file)?)?)
    } else {
        Default::default()
    };

    log::info!("Started controller on {bind_address}. Config: {config:?}");
    let stats = Arc::new(ClientStatistics::new(&config));

    let (broadcaster, rx) = bounded(Some((config.clone(), stats.clone())), 1);

    let mut controller = Controller {
        current: (config, stats),
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
