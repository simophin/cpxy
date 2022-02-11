use crate::abp::{update_abp_list, update_gfw_list};
use crate::broadcast::bounded;
use crate::client::{run_client, ClientStatistics};
use crate::config::{ClientConfig, UpstreamConfig};
use crate::http::{parse_request, write_http_response};
use crate::io::TcpListener;
use anyhow::Context;
use async_broadcast::Sender;
use futures_lite::io::split;
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use smol::fs::File;
use smol::spawn;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "web/build"]
struct Asset;

enum Response {
    Empty,
    Json(Vec<u8>),
    EmbedFile {
        path: String,
        file: rust_embed::EmbeddedFile,
    },
}

fn json_response(a: impl Serialize) -> Result<Response, ErrorResponse> {
    Ok(Response::Json(
        serde_json::to_vec(&a).map_err(|e| ErrorResponse::Generic(e.into()))?,
    ))
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
        let result = match parse_request(r, Default::default()).await {
            Ok(mut r) => match (r.method.as_str(), r.path.as_str()) {
                ("options", _) => Ok(Response::Empty),
                ("get", p) if p.starts_with("/api/config") => {
                    self.get_config().and_then(json_response)
                }
                ("post", p) if p.starts_with("/api/config") => self
                    .set_basic_config(r.body_json().await?)
                    .await
                    .and_then(json_response),
                ("get", p) if p.starts_with("/api/stats") => {
                    self.get_stats().and_then(json_response)
                }
                ("get", original_p) => {
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
                ("post", p) if p.starts_with("/api/gfwlist") => {
                    update_gfw_list(&self.current.0.socks5_address)
                        .await
                        .map_err(|e| ErrorResponse::Generic(e))
                        .and_then(|num| json_response(RuleUpdateResult { num_updated: num }))
                }
                ("post", p) if p.starts_with("/api/abplist") => {
                    update_abp_list(&self.current.0.socks5_address)
                        .await
                        .map_err(|e| ErrorResponse::Generic(e))
                        .and_then(|num| json_response(RuleUpdateResult { num_updated: num }))
                }
                ("post", p) if p.starts_with("/api/upstream") => self
                    .update_upstreams(r.body_json().await?)
                    .await
                    .and_then(json_response),
                ("delete", p) if p.starts_with("/api/upstream") => self
                    .delete_upstreams(r.body_json().await?)
                    .await
                    .and_then(json_response),
                _ => Err(ErrorResponse::NotFound(r.path.clone())),
            },
            Err((e, _)) => Err(ErrorResponse::InvalidRequest(e)),
        };

        match result {
            Ok(Response::Empty) => write_http_response(&mut w, 201, Some("OK"), None, &[]).await?,
            Ok(Response::Json(buf)) => {
                write_http_response(
                    &mut w,
                    200,
                    Some("OK"),
                    Some("application/json"),
                    buf.as_slice(),
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
