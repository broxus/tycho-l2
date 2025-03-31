use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use sync_service::config::Config;
use sync_service::service::Uploader;

#[derive(Parser)]
pub struct Cmd {
    // Path to the service config.
    #[clap(long)]
    pub config: PathBuf,
}

impl Cmd {
    pub async fn run(self) -> Result<()> {
        let config = Config::load_from_file(self.config)?;
        anyhow::ensure!(!config.workers.is_empty(), "no workers specified");

        let mut uploaders = Vec::new();
        for worker in config.workers {
            let left_client = worker.left.client.build_client()?;
            let right_client = worker.right.client.build_client()?;

            tracing::info!(
                src = left_client.name(),
                dst = right_client.name(),
                "starting uploader",
            );
            let uploader = Uploader::new(
                left_client.clone(),
                right_client.clone(),
                worker.right.uploader.clone(),
            )
            .await?;
            uploaders.push(uploader);

            tracing::info!(
                src = right_client.name(),
                dst = left_client.name(),
                "starting uploader",
            );
            let uploader =
                Uploader::new(right_client, left_client, worker.left.uploader.clone()).await?;
            uploaders.push(uploader);
        }
        tracing::info!("all uploaders created");

        for uploader in uploaders {
            tokio::task::spawn(uploader.run());
        }
        tracing::info!("all uploaders started");

        futures_util::future::pending().await
    }
}
