use crate::abp::{adblock_list_engine, gfw_list_engine};
use crate::broadcast::bounded;
use crate::buf::RWBuffer;
use crate::client::{run_client, ClientStatistics};
use crate::config::{ClientConfig, UpstreamConfig};
use crate::http::{parse_request, write_http_response, WithHeaders};
use crate::http_path::HttpPath;
use crate::socks5::Address;
use anyhow::{anyhow, Context};
use async_broadcast::Sender;
use async_net::TcpListener;
use chrono::{DateTime, Utc};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
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
    Regular {
        data: Vec<u8>,
        mime_type: String,
    },
    EmbedFile {
        path: String,
        file: rust_embed::EmbeddedFile,
    },
}

impl Response {
    pub fn mapper<T: Serialize>(
        mime_type: impl AsRef<str> + Into<String>,
    ) -> impl FnOnce(T) -> Result<Self, ErrorResponse> {
        move |data| {
            let (data, mime_type) = if mime_type.as_ref().eq_ignore_ascii_case("application/json")
                || mime_type.as_ref().eq_ignore_ascii_case("*/*")
            {
                (
                    serde_json::to_vec(&data).context("Serialising JSON"),
                    "application/json",
                )
            } else if mime_type.as_ref().eq_ignore_ascii_case("text/yaml") {
                (
                    serde_yaml::to_string(&data)
                        .context("Serialising YAML")
                        .map(|s| s.into_bytes()),
                    "text/yaml",
                )
            } else {
                (
                    Err(anyhow!("Invalid mimetype: {}", mime_type.as_ref())),
                    "text/plain",
                )
            };

            if let Err(e) = &data {
                log::error!("Error serialising response: {e}");
            }

            data.map(|data| Response::Regular {
                data,
                mime_type: mime_type.into(),
            })
            .map_err(ErrorResponse::Generic)
        }
    }
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
struct RuleResult {
    num_rules: Option<usize>,
    last_updated: Option<DateTime<Utc>>,
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

    async fn set_config(&mut self, mut c: ClientConfig, replace_upstreams: bool) -> HttpResult<()> {
        let (ClientConfig { upstreams, .. }, new_stats) = (
            self.current.0.as_ref().clone(),
            self.current.1.as_ref().clone(),
        );

        if !replace_upstreams {
            c.upstreams = upstreams;
        }

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
                    Some(upstream) if upstream.protocol == config.protocol => {
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

    async fn dispatch(&mut self, r: impl AsyncRead + Unpin + Send + Sync) -> HttpResult<Response> {
        match parse_request(r, RWBuffer::new_vec_uninitialised(512)).await {
            Ok(mut r) => {
                let path: HttpPath = r.path.parse()?;
                log::debug!("Dispatching {} {}", r.method, r.path);
                let mime_type = if r.method.as_ref().eq_ignore_ascii_case("get") {
                    r.get_header_text("Accept")
                } else {
                    r.get_header_text("Content-Type")
                }
                .unwrap_or("application/json")
                .to_string();

                match (r.method.as_ref(), path.path.as_ref()) {
                    ("OPTIONS", _) => Ok(Response::Empty),
                    ("GET", "/api/config") => {
                        self.get_config().and_then(Response::mapper(mime_type))
                    }
                    ("POST", "/api/config") => self
                        .set_config(
                            r.body_json_or_yaml().await?,
                            path.get_query("replace") == Some("all"),
                        )
                        .await
                        .and_then(Response::mapper(mime_type)),
                    ("GET", "/api/stats") => self.get_stats().and_then(Response::mapper(mime_type)),
                    (m, "/api/gfwlist") | (m, "/api/adblocklist") => {
                        let engine = if path.path.contains("gfwlist") {
                            gfw_list_engine()
                        } else {
                            adblock_list_engine()
                        };

                        match m {
                            "GET" => engine
                                .get_last_updated()
                                .map(|last_updated| RuleResult {
                                    last_updated,
                                    num_rules: None,
                                })
                                .map_err(|e| ErrorResponse::Generic(e))
                                .and_then(Response::mapper(mime_type)),
                            "POST" => engine
                                .update(&Address::IP(self.current.0.socks5_address))
                                .await
                                .and_then(|num_rules| {
                                    Ok(RuleResult {
                                        num_rules: Some(num_rules),
                                        last_updated: engine.get_last_updated()?,
                                    })
                                })
                                .map_err(|e| ErrorResponse::Generic(e))
                                .and_then(Response::mapper(mime_type)),
                            _ => Err(ErrorResponse::InvalidRequest(anyhow!("Unknown method {m}"))),
                        }
                    }

                    // This MUST BE the last GET resource
                    ("GET", original_p) => {
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
                    ("POST", "/api/upstream") => self
                        .update_upstreams(r.body_json_or_yaml().await?)
                        .await
                        .and_then(Response::mapper(mime_type)),
                    ("DELETE", "/api/upstream") => self
                        .delete_upstreams(r.body_json_or_yaml().await?)
                        .await
                        .and_then(Response::mapper(mime_type)),
                    (m, p) => {
                        log::warn!("Request {m} {p} not found");
                        Err(ErrorResponse::NotFound(p.to_string()))
                    }
                }
            }
            Err((e, _)) => Err(ErrorResponse::InvalidRequest(e)),
        }
    }

    async fn handle_client(
        &mut self,
        sock: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    ) -> anyhow::Result<()> {
        let (r, mut w) = sock.split();
        let result = self.dispatch(r).await;

        match result {
            Ok(Response::Empty) => write_http_response(&mut w, 201, Some("OK"), None, &[]).await?,
            Ok(Response::Regular { data, mime_type }) => {
                write_http_response(&mut w, 200, Some("OK"), Some(&mime_type), data.as_ref())
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

    let _client_task = spawn(run_client(rx));

    loop {
        let (socket, addr) = listener.accept().await?;
        log::debug!("Serving controller client: {addr}");
        match controller.handle_client(socket).await {
            Ok(_) => {}
            Err(e) => {
                log::error!("Error serving client {addr}: {e:?}");
            }
        }
        log::debug!("Client {addr} disconnected");
    }
}
