use crate::broadcast::bounded;
use crate::client::{run_client, ClientStatistics};
use crate::config::ClientConfig;
use crate::io::TcpListener;
use crate::socks5::Address;
use crate::utils::{JsonSerializable, RWBuffer};
use anyhow::anyhow;
use async_broadcast::Sender;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use rust_embed::RustEmbed;
use serde::de::DeserializeOwned;
use smol::fs::File;
use smol::spawn;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

struct Controller {
    current: (Arc<ClientConfig>, Arc<ClientStatistics>),
    broadcaster: Sender<(Arc<ClientConfig>, Arc<ClientStatistics>)>,
    config_file: PathBuf,
}

fn parse_json<T: DeserializeOwned>(s: &RWBuffer) -> Result<T, ErrorResponse> {
    serde_json::from_slice(s.read_buf()).map_err(|e| ErrorResponse::Generic(e.into()))
}

struct HttpParseResult {
    method: String,
    path: String,
    is_json: bool,
    body: RWBuffer,
}

async fn parse_request(
    input: &mut (impl AsyncRead + Unpin + Send + Sync),
) -> anyhow::Result<HttpParseResult> {
    let mut buf = RWBuffer::with_capacity(65536);
    loop {
        match input.read(buf.write_buf()).await? {
            0 => return Err(anyhow!("Unexpected EOF")),
            v => buf.advance_write(v),
        };

        let mut headers = [httparse::EMPTY_HEADER; 20];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf.read_buf())? {
            httparse::Status::Complete(offset) => {
                let method = req.method.unwrap_or("").to_ascii_lowercase();
                let path = req.path.unwrap_or("").to_ascii_lowercase();

                let content_length = req
                    .headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case("content-length"))
                    .and_then(|h| String::from_utf8_lossy(h.value).parse().ok())
                    .unwrap_or(0usize);

                let content_type = req
                    .headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case("content-type"))
                    .map(|h| String::from_utf8_lossy(h.value).to_ascii_lowercase());

                let is_json = match content_type {
                    Some(c) if c.starts_with("application/json") => true,
                    _ => false,
                };

                drop(req);
                drop(headers);

                buf.advance_read(offset);
                buf.compact();

                if buf.capacity() < content_length {
                    return Err(anyhow!("Payload too big"));
                }

                if content_length > buf.remaining_read() {
                    let to_read = content_length - buf.remaining_read();
                    input.read_exact(&mut buf.write_buf()[..to_read]).await?;
                    buf.advance_write(to_read);
                }

                return Ok(HttpParseResult {
                    path,
                    method,
                    is_json,
                    body: buf,
                });
            }
            httparse::Status::Partial => {}
        }
    }
}

#[derive(RustEmbed)]
#[folder = "$OUT_DIR/webapp"]
struct Asset;

enum Response {
    Empty,
    Json(Box<dyn JsonSerializable>),
    EmbedFile {
        path: String,
        file: rust_embed::EmbeddedFile,
    },
}

enum ErrorResponse {
    Generic(anyhow::Error),
    NotFound(String),
    InvalidRequest(anyhow::Error),
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
            _ => {}
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
        let result = match parse_request(&mut sock).await {
            Ok(HttpParseResult {
                method,
                path,
                is_json,
                body,
            }) => match (method.as_str(), path.as_str(), is_json) {
                ("options", _, _) => Ok(Response::Empty),
                ("get", p, _) if p.starts_with("/api/config") => {
                    self.get_config().map(|r| Response::Json(Box::new(r)))
                }
                ("get", p, _) if p.starts_with("/api/stats") => {
                    self.get_stats().map(|r| Response::Json(Box::new(r)))
                }
                ("get", original_p, _) => {
                    let p = if original_p == "/" || original_p.is_empty() {
                        "index.html"
                    } else {
                        &original_p[1..]
                    };
                    match Asset::get(p) {
                        Some(file) => Ok(Response::EmbedFile {
                            path: p.to_string(),
                            file,
                        }),
                        _ => Err(ErrorResponse::NotFound(original_p.to_string())),
                    }
                }
                ("post", p, true) if p.starts_with("/api/config") => match parse_json(&body) {
                    Ok(body) => self
                        .set_config(body)
                        .await
                        .map(|r| Response::Json(Box::new(r))),
                    Err(e) => Err(e),
                },
                _ => Err(ErrorResponse::NotFound(path)),
            },
            Err(e) => Err(ErrorResponse::InvalidRequest(e)),
        };

        match result {
            Ok(Response::Empty) => {
                sock.write_all(
                    b"HTTP/1.1 201\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Access-Control-Allow-Headers: *\r\n\
                \r\n",
                )
                .await?;
            }
            Ok(Response::Json(buf)) => {
                let buf = buf.to_json();
                sock.write_all(
                    format!(
                        "HTTP/1.1 200\r\n\
                    Content-Type: application/json\r\n\
                    Access-Control-Allow-Origin: *\r\n\
                    Access-Control-Allow-Headers: *\r\n\
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
                Access-Control-Allow-Headers: *\r\n\
                Access-Control-Allow-Origin: *\r\n\
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
                    ErrorResponse::InvalidRequest(e) => (400, e.to_string()),
                    ErrorResponse::NotFound(s) => (404, format!("Resource {s} not found")),
                };
                let msg = msg.as_bytes();
                sock.write_all(
                    format!(
                        "HTTP/1.1 {code}\r\n\
                    Content-Type: text/plain\r\n\
                    Content-Length: {}\r\n\
                    Access-Control-Allow-Headers: *\r\n\
                    Access-Control-Allow-Origin: *\r\n\
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

pub async fn run_controller(
    bind_address: SocketAddr,
    config_file: &std::path::Path,
) -> anyhow::Result<()> {
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
