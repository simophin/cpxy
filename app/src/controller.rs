use crate::abp::{update_abp_list, update_gfw_list};
use crate::broadcast::bounded;
use crate::client::{run_client, ClientStatistics};
use crate::config::{ClientConfig, UpstreamConfig};
use crate::http::HttpRequest;
use crate::io::TcpListener;
use crate::utils::{write_http_response, JsonSerializable, RWBuffer};
use anyhow::{bail, Context};
use async_broadcast::Sender;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use rust_embed::RustEmbed;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use smol::fs::File;
use smol::spawn;
use std::path::PathBuf;
use std::sync::Arc;

fn parse_json<T: DeserializeOwned>(s: &impl AsRef<[u8]>) -> Result<T, ErrorResponse> {
    serde_json::from_slice(s.as_ref()).map_err(|e| ErrorResponse::Generic(e.into()))
}

struct HttpParseResult {
    method: String,
    path: String,
    is_json: bool,
    body: Vec<u8>,
}

async fn parse_request(
    input: impl AsyncRead + Unpin + Send + Sync,
) -> anyhow::Result<HttpParseResult> {
    let mut req = super::http::parse_request(input, RWBuffer::with_capacity(65536)).await?;
    let is_json = match req.get_content_type() {
        Some(value) if value.starts_with("application/json") => true,
        _ => false,
    };

    let content_length = req.get_content_length().unwrap_or_default();

    if content_length > 65536 {
        bail!("Payload is too big");
    }

    let body = if content_length > 0 {
        let mut buf = Vec::with_capacity(content_length);
        req.read_exact(&mut buf).await?;
        buf
    } else {
        Vec::new()
    };

    let HttpRequest { method, path, .. } = req.into_init();

    Ok(HttpParseResult {
        method,
        path,
        is_json,
        body,
    })
}

#[derive(RustEmbed)]
#[folder = "web/build"]
struct Asset;

enum Response {
    Empty,
    Json(Box<dyn JsonSerializable + Send + Sync>),
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

impl From<anyhow::Error> for ErrorResponse {
    fn from(e: anyhow::Error) -> Self {
        Self::Generic(e)
    }
}

type HttpResult<T> = Result<T, ErrorResponse>;

#[derive(Serialize)]
struct RuleUpdateResult {
    num_updated: usize,
}

struct Controller {
    current: (Arc<ClientConfig>, Arc<ClientStatistics>),
    broadcaster: Sender<(Arc<ClientConfig>, Arc<ClientStatistics>)>,
    config_file: PathBuf,
}

#[derive(Deserialize)]
struct UpstreamUpdate {
    old_name: Option<String>,
    name: String,
    config: UpstreamConfig,
}

impl Controller {
    fn get_stats(&self) -> HttpResult<Arc<ClientStatistics>> {
        Ok(self.current.1.clone())
    }

    fn get_config(&self) -> HttpResult<Arc<ClientConfig>> {
        Ok(self.current.0.clone())
    }

    async fn set_current_config(&mut self, c: ClientConfig, s: ClientStatistics) -> HttpResult<()> {
        let stats = Arc::new(s);
        let config = Arc::new(c);

        let config_text = serde_yaml::to_string(config.as_ref())
            .with_context(|| format!("Writing YAML file: {:?}", self.config_file))?;

        let mut file = File::create(&self.config_file)
            .await
            .with_context(|| format!("Creating configuration file: {:?}", self.config_file))?;

        let _ = file
            .write_all(config_text.as_bytes())
            .await
            .with_context(|| format!("Writing configuration file: {:?}", self.config_file))?;

        file.flush()
            .await
            .with_context(|| format!("Flushing file: {:?}", self.config_file))?;

        log::info!("Config written successfully to {:?}", self.config_file);
        self.current = (config.clone(), stats.clone());
        let _ = self.broadcaster.broadcast((config.clone(), stats)).await;
        Ok(())
    }

    async fn set_basic_config(&mut self, mut c: ClientConfig) -> HttpResult<()> {
        let (ClientConfig { upstreams, .. }, new_stats) = (
            self.current.0.as_ref().clone(),
            self.current.1.as_ref().clone(),
        );

        c.upstreams = upstreams;

        self.set_current_config(c, new_stats).await
    }

    async fn update_upstreams(&mut self, updates: Vec<UpstreamUpdate>) -> HttpResult<()> {
        let (mut new_config, mut new_stats) = (
            self.current.0.as_ref().clone(),
            self.current.1.as_ref().clone(),
        );
        for UpstreamUpdate {
            old_name,
            name,
            config,
        } in updates
        {
            let stats = if let Some(old_name) = old_name {
                match new_config.upstreams.remove(&old_name) {
                    Some(upstream) if upstream.address == config.address => {
                        // Reuse the stats
                        new_stats.upstreams.remove(&old_name)
                    }
                    Some(_) => {
                        new_stats.upstreams.remove(&old_name);
                        None
                    }
                    _ => None,
                }
            } else {
                None
            };

            new_config.upstreams.insert(name.clone(), config);
            new_stats.upstreams.insert(name, stats.unwrap_or_default());
        }

        self.set_current_config(new_config, new_stats).await
    }

    async fn delete_upstreams(&mut self, names: Vec<String>) -> HttpResult<()> {
        let (mut new_config, mut new_stats) = (
            self.current.0.as_ref().clone(),
            self.current.1.as_ref().clone(),
        );
        for n in names {
            let _ = new_config.upstreams.remove(&n);
            let _ = new_stats.upstreams.remove(&n);
        }
        self.set_current_config(new_config, new_stats).await
    }

    async fn handle_client(
        &mut self,
        sock: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    ) -> anyhow::Result<()> {
        let (r, mut w) = split(sock);
        let result = match parse_request(r).await {
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
                ("post", p, true) if p.starts_with("/api/config") => match parse_json(&body) {
                    Ok(body) => self
                        .set_basic_config(body)
                        .await
                        .map(|r| Response::Json(Box::new(r))),
                    Err(e) => Err(e),
                },
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
                ("post", p, _) if p.starts_with("/api/gfwlist") => {
                    update_gfw_list(&self.current.0.socks5_address)
                        .await
                        .map(|num| Response::Json(Box::new(RuleUpdateResult { num_updated: num })))
                        .map_err(|e| ErrorResponse::Generic(e))
                }
                ("post", p, _) if p.starts_with("/api/abplist") => {
                    update_abp_list(&self.current.0.socks5_address)
                        .await
                        .map(|num| Response::Json(Box::new(RuleUpdateResult { num_updated: num })))
                        .map_err(|e| ErrorResponse::Generic(e))
                }
                ("post", p, true) if p.starts_with("/api/upstream") => match parse_json(&body) {
                    Ok(body) => self
                        .update_upstreams(body)
                        .await
                        .map(|r| Response::Json(Box::new(r))),
                    Err(e) => Err(e),
                },
                ("delete", p, true) if p.starts_with("/api/upstream") => match parse_json(&body) {
                    Ok(body) => self
                        .delete_upstreams(body)
                        .await
                        .map(|r| Response::Json(Box::new(r))),
                    Err(e) => Err(e),
                },
                _ => Err(ErrorResponse::NotFound(path)),
            },
            Err(e) => Err(ErrorResponse::InvalidRequest(e)),
        };

        match result {
            Ok(Response::Empty) => write_http_response(&mut w, 201, Some("OK"), None, &[]).await?,
            Ok(Response::Json(buf)) => {
                write_http_response(
                    &mut w,
                    200,
                    Some("OK"),
                    Some("application/json"),
                    buf.to_json().as_slice(),
                )
                .await?
            }
            Ok(Response::EmbedFile { path, file }) => {
                let mime = mime_guess::from_path(path);
                write_http_response(
                    &mut w,
                    200,
                    Some("OK"),
                    mime.first_raw(),
                    file.data.as_ref(),
                )
                .await?
            }
            Err(e) => {
                let (code, msg) = match e {
                    ErrorResponse::Generic(e) => (500, e.to_string()),
                    ErrorResponse::InvalidRequest(e) => (400, e.to_string()),
                    ErrorResponse::NotFound(s) => (404, format!("Resource {s} not found")),
                };
                write_http_response(&mut w, code, None, Some("text/plain"), msg.as_bytes()).await?
            }
        };
        Ok(())
    }
}

pub async fn run_controller(
    listener: TcpListener,
    config_file: &std::path::Path,
) -> anyhow::Result<()> {
    let config = if config_file.exists() {
        Arc::new(
            serde_yaml::from_reader(
                std::fs::File::open(config_file)
                    .with_context(|| format!("Opening config file {config_file:?}"))?,
            )
            .with_context(|| format!("Parsing config file {config_file:?}"))?,
        )
    } else {
        Default::default()
    };

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
                log::error!("Error serving client {addr}: {e:?}");
            }
        }
        log::info!("Client {addr} disconnected");
    }
}
