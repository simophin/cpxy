use async_trait::async_trait;

use crate::{broadcast, config::ClientConfig};

pub mod fs;

#[async_trait]
pub trait ConfigProvider: Sized {
    type Settings;

    async fn new(
        settings: Self::Settings,
    ) -> anyhow::Result<(Self, broadcast::Receiver<ClientConfig>)>;

    async fn update_config(&mut self, config: &ClientConfig) -> anyhow::Result<()>;
}
