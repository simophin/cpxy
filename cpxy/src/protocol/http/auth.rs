use anyhow::{bail, Context};
use async_trait::async_trait;
use base64::display::Base64Display;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait AuthProvider: Send + Sync {
    async fn check_auth(&self, value: Option<&str>) -> anyhow::Result<()>;
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
pub struct BasicAuthSettings {
    pub user: String,
    pub password: String,
}

impl BasicAuthSettings {
    pub fn to_header_value(&self) -> String {
        use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
        let credentials = format!("{}:{}", self.user, self.password);

        format!("Basic {}", Base64Display::new(credentials.as_bytes(), &B64))
    }
}

#[derive(Clone)]
pub struct BasicAuthProvider(pub BasicAuthSettings);

#[async_trait]
impl AuthProvider for BasicAuthProvider {
    async fn check_auth(&self, value: Option<&str>) -> anyhow::Result<()> {
        let value = value.context("Expecting auth")?;
        if value.trim() != self.0.to_header_value().trim() {
            bail!("Invalid password")
        }

        Ok(())
    }
}
