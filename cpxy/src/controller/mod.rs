mod api;
mod fs_provider;

use crate::config::ClientConfig;
use async_trait::async_trait;

#[async_trait]
pub trait ConfigProvider {
    async fn update_config(&mut self, config: &ClientConfig) -> anyhow::Result<()>;
}

pub struct Controller {}
