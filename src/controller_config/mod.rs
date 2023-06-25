use async_trait::async_trait;

use crate::config::ClientConfig;
use tokio::sync::watch;

pub mod fs;

#[async_trait]
pub trait ConfigProvider: Sized {
    type Settings;

    async fn new(settings: Self::Settings)
        -> anyhow::Result<(Self, watch::Receiver<ClientConfig>)>;

    async fn update_config(&mut self, config: &ClientConfig) -> anyhow::Result<()>;
}
