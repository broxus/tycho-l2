use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::client::ClientConfig;
use crate::service::UploaderConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub workers: Vec<WorkerConfig>,
}

impl Config {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let data = std::fs::read(path).context("failed to read service config")?;
        serde_json::from_slice(&data).context("failed to deserialize service config")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    pub left: NetworkConfig,
    pub right: NetworkConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    #[serde(flatten)]
    pub client: ClientConfig,
    pub uploader: UploaderConfig,
}
