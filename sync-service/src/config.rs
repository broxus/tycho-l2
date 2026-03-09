use std::path::Path;

use anyhow::Context;
use serde::Deserialize;
use tycho_util::cli::metrics::MetricsConfig;

use crate::client::ClientConfig;
use crate::service::UploaderConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub metrics: Option<MetricsConfig>,
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
    pub uploader: Option<UploaderConfig>,
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn parses_without_metrics() {
        let config: Config = serde_json::from_str(
            r#"{
                "workers": []
            }"#,
        )
        .unwrap();

        assert!(config.metrics.is_none());
    }

    #[test]
    fn parses_metrics() {
        let config: Config = serde_json::from_str(
            r#"{
                "metrics": {
                    "listen_addr": "0.0.0.0:10000"
                },
                "workers": []
            }"#,
        )
        .unwrap();

        let metrics = config.metrics.unwrap();
        assert_eq!(metrics.listen_addr.to_string(), "0.0.0.0:10000");
    }
}
