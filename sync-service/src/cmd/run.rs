use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use sync_service::config::Config;
use sync_service::metrics::UploaderMetricsState;
use sync_service::service::Uploader;
use tycho_util::cli::metrics::init_metrics;

#[derive(Parser)]
pub struct Cmd {
    // Path to the service config.
    #[clap(long)]
    pub config: PathBuf,
}

impl Cmd {
    pub async fn run(self) -> Result<()> {
        let Config { metrics, workers } = Config::load_from_file(self.config)?;
        anyhow::ensure!(!workers.is_empty(), "no workers specified");
        if let Some(metrics) = &metrics {
            init_metrics(metrics)?;
        }

        let mut uploaders = Vec::new();
        for worker in workers {
            let left_client = worker.left.client.build_client()?;
            let right_client = worker.right.client.build_client()?;

            if let Some(uploader) = worker.right.uploader.clone() {
                let metrics = UploaderMetricsState::new(
                    left_client.name(),
                    right_client.name(),
                    uploader.wallet_address.to_string(),
                );
                tracing::info!(
                    src = left_client.name(),
                    dst = right_client.name(),
                    "starting uploader",
                );
                let u = Uploader::new(
                    left_client.clone(),
                    right_client.clone(),
                    metrics.clone(),
                    uploader,
                )
                .await?;
                uploaders.push(u);
            }

            if let Some(uploader) = worker.left.uploader.clone() {
                let metrics = UploaderMetricsState::new(
                    right_client.name(),
                    left_client.name(),
                    uploader.wallet_address.to_string(),
                );
                tracing::info!(
                    src = right_client.name(),
                    dst = left_client.name(),
                    "starting uploader",
                );
                let u = Uploader::new(right_client, left_client, metrics.clone(), uploader).await?;
                uploaders.push(u);
            }
        }
        tracing::info!("all uploaders created");

        for uploader in uploaders {
            tokio::task::spawn(uploader.run());
        }
        tracing::info!("all uploaders started");

        futures_util::future::pending().await
    }
}
