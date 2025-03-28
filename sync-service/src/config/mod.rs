use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::provider::BlockProviderConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceConfig {
    pub workers: Vec<WorkerType>,
}

impl ServiceConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let data = std::fs::read(path).context("failed to read service config")?;
        serde_json::from_slice(&data).context("failed to deserialize service config")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub enum WorkerType {
    TonL2(WorkerConfig),
    L2Ton(WorkerConfig),
    L2L2(L2L2WorkerConfig),
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    pub l2_rcp_url: String,
    pub block_provider: BlockProviderConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct L2L2WorkerConfig {
    pub left_rcp_url: String,
    pub right_rcp_url: String,
    pub block_provider: BlockProviderConfig,
}

pub trait WorkerConfigExt {
    fn block_provider(&self) -> BlockProviderConfig;
}

impl WorkerConfigExt for WorkerConfig {
    fn block_provider(&self) -> BlockProviderConfig {
        self.block_provider.clone()
    }
}

impl WorkerConfigExt for L2L2WorkerConfig {
    fn block_provider(&self) -> BlockProviderConfig {
        self.block_provider.clone()
    }
}
