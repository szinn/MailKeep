use std::path::PathBuf;

use mk_frontend::{FrontendConfig, OidcConfig};
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
    #[serde(default = "default_job_concurrency")]
    pub job_concurrency: usize,
    /// Interval (seconds) between IMAP poll passes over non-IDLE folders.
    /// Env: `MAILKEEP__IMAP_POLL_INTERVAL_SECS`.
    #[serde(default = "default_imap_poll_interval_secs")]
    pub imap_poll_interval_secs: u64,
}

fn default_job_concurrency() -> usize {
    1
}

fn default_imap_poll_interval_secs() -> u64 {
    300
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
