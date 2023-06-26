use async_trait::async_trait;

use crate::config::ClientConfig;

pub mod fs;

#[async_trait]
pub trait ConfigProvider {
    async fn update_config(&mut self, config: &ClientConfig) -> anyhow::Result<()>;
}
