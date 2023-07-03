use anyhow::{bail, Context};
use base64::display::Base64Display;
use serde::{Deserialize, Serialize};

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

impl BasicAuthProvider {
    pub fn check_auth(&self, value: Option<impl AsRef<str>>) -> anyhow::Result<()> {
        let value = value.context("Expecting auth")?;
        if value.as_ref().trim() != self.0.to_header_value().trim() {
            bail!("Invalid password")
        }

        Ok(())
    }
}
