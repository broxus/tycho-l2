use std::path::Path;

use anyhow::Context;
use everscale_types::models::StdAddr;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceConfig {
    pub l2_ton: Vec<WorkerConfig>,
    pub ton_l2: Vec<WorkerConfig>,
}

impl ServiceConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let data = std::fs::read(path).context("failed to read service config")?;
        serde_json::from_slice(&data).context("failed to deserialize service config")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    pub tycho_rcp_url: String,
    pub bridge_address: StdAddr,
}
