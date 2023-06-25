use super::ConfigProvider;
use crate::config::ClientConfig;
use anyhow::Context;
use async_trait::async_trait;
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

pub struct Settings {
    pub path: PathBuf,
}

pub struct FileConfigProvider {
    path: PathBuf,
    sender: watch::Sender<ClientConfig>,
}

async fn read_config(path: impl AsRef<Path>) -> anyhow::Result<ClientConfig> {
    let path = path.as_ref();
    let file = File::open(&path).with_context(|| format!("Opening file {}", path.display()))?;
    let file = BufReader::new(file);

    if path.ends_with(".json") {
        serde_json::from_reader(file).context("Deserialising config")
    } else {
        serde_yaml::from_reader(file).context("Deserialising config")
    }
}

#[async_trait]
impl ConfigProvider for FileConfigProvider {
    type Settings = Settings;

    async fn new(
        Settings { path }: Settings,
    ) -> anyhow::Result<(Self, watch::Receiver<ClientConfig>)> {
        let config = read_config(&path).await?;
        let (sender, receiver) = watch::channel(config);
        Ok((Self { path, sender }, receiver))
    }

    async fn update_config(&mut self, config: &ClientConfig) -> anyhow::Result<()> {
        todo!()
    }
}
