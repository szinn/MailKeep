use std::path::PathBuf;

use serde::Deserialize;

use crate::error::Error;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub metadata_path: PathBuf,
    pub storage_path: PathBuf,
    pub encryption_secret: String,
    #[serde(default)]
    pub frontend: FrontendConfig,
    #[serde(default)]
    pub oidc: OidcConfig,
}

impl Config {
    pub fn load() -> Result<Self, Error> {
        let config = config::Config::builder()
            .add_source(config::Environment::with_prefix("MAILKEEP").try_parsing(true).separator("__"))
            .build()?;

        let config = config.try_deserialize()?;

        Ok(config)
    }
}
