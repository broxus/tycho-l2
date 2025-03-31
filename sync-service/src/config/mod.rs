use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::provider::BlockProviderConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceConfig {
    pub workers: Vec<WorkerConfig>,
}

impl ServiceConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let data = std::fs::read(path).context("failed to read service config")?;
        serde_json::from_slice(&data).context("failed to deserialize service config")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    pub left_client: ClientType,
    pub right_client: ClientType,
    pub block_provider: BlockProviderConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub enum ClientType {
    Ton,
    Tycho { url: String },
}

impl std::fmt::Display for ClientType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ton => write!(f, "Ton"),
            Self::Tycho { .. } => write!(f, "L2"),
        }
    }
}
