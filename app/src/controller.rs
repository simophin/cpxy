use crate::client::{ClientStatistics, UpstreamStatistics};
use crate::config::{ClientConfig, UpstreamConfig};
use async_broadcast::Sender;
use futures_lite::{AsyncRead, AsyncWrite, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use smol::fs::File;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

struct Controller {
    current: RwLock<(Arc<ClientConfig>, Arc<ClientStatistics>)>,
    broadcaster: Sender<(Arc<ClientConfig>, Arc<ClientStatistics>)>,
    config_file: PathBuf,
}

#[derive(Default, Serialize, Debug)]
struct ConfigWithStats {
    config: Arc<ClientConfig>,
    stats: Arc<ClientStatistics>,
}

impl Controller {
    fn get_config_state(&self) -> anyhow::Result<ConfigWithStats> {
        let (config, stats) = match self.current.try_read() {
            Ok(v) => (v.0.clone(), v.1.clone()),
            Err(e) => return Err(anyhow::anyhow!("Error locking current: {e}")),
        };
        Ok(ConfigWithStats { config, stats })
    }

    async fn set_config(&self, new_config: ClientConfig) -> anyhow::Result<ConfigWithStats> {
        let config_with_stats = self.get_config_state()?;
        if config_with_stats.config.as_ref() == new_config {
            return Ok(config_with_stats);
        }
        let stats = Arc::new(ClientStatistics {
            upstreams: new_config
                .upstreams
                .iter()
                .map(|(name, _)| (name, Default::default()))
                .collect(),
        });

        let config = Arc::new(new_config);

        match self.current.write() {
            Ok(g) => *g = (config.clone(), stats.clone()),
            Err(e) => return Err(e.into()),
        };

        let _ = self
            .broadcaster
            .broadcast((config.clone(), stats.clone()))
            .await;

        let _ = File::create(&self.config_file)
            .await?
            .write_all(serde_json::to_vec(config.as_ref())?.as_slice())
            .await?;
        log::info!("Config written successfully to {:?}", self.config_file);
    }

    async fn handle_client(
        sock: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    ) -> anyhow::Result<()> {
        let mut buf = vec![0u8; 1024];
    }
}

pub async fn run_controller(bind_address: SocketAddr, config_file: &Path) -> anyhow::Result<()> {
    todo!()
}
