use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use sync_service::config::{ClientType, ServiceConfig};
use sync_service::provider::KeyBlockProviderClient;
use sync_service::uploader::KeyBlockUploaderClient;
use sync_service::utils::jrpc_client::JrpcClient;
use tokio::task::JoinSet;
use ton_lite_client::{LiteClient, LiteClientConfig, TonGlobalConfig};

use crate::service::ServiceWorker;

#[derive(Parser)]
pub struct Cmd {
    // Path to the TON global config.
    #[clap(long)]
    pub global_config: PathBuf,

    // Path to the Service config.
    #[clap(long)]
    pub service_config: PathBuf,
}

impl Cmd {
    pub async fn run(self) -> Result<()> {
        let global_config = TonGlobalConfig::load_from_file(self.global_config)?;
        let ton_lite_client =
            LiteClient::new(LiteClientConfig::default(), global_config.liteservers);

        let service_config = ServiceConfig::load_from_file(self.service_config)?;

        let mut handles = JoinSet::new();
        for config in service_config.workers {
            let left_client: Box<dyn KeyBlockProviderClient + Send + Sync> =
                match &config.left_client {
                    ClientType::Ton => Box::new(ton_lite_client.clone()),
                    ClientType::Tycho { url } => Box::new(JrpcClient::new(url.parse()?)?),
                };

            let right_client: Box<dyn KeyBlockUploaderClient + Send + Sync> =
                match &config.right_client {
                    ClientType::Ton => Box::new(ton_lite_client.clone()),
                    ClientType::Tycho { url } => Box::new(JrpcClient::new(url.parse()?)?),
                };

            let worker_name = format!("{}->{}", config.right_client, config.right_client);
            let worker = ServiceWorker::new(left_client, right_client, config).await?;

            handles.spawn(async move {
                tracing::info!("worker {} started", worker_name);
                if let Err(e) = worker.run().await {
                    tracing::info!("worker {} failed: {e}", worker_name);
                }
                tracing::info!("worker {} finished", worker_name);

                worker_name
            });
        }

        while let Some(result) = handles.join_next().await {
            match result {
                Ok(worker) => tracing::warn!("worker {worker} completed"),
                Err(e) => tracing::error!("worker failed: {e}"),
            }
        }

        Ok(())
    }
}
