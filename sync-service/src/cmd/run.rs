use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use sync_service::config::{ServiceConfig, WorkerType};
use sync_service::utils::jrpc_client::JrpcClient;
use ton_lite_client::{LiteClient, LiteClientConfig, TonGlobalConfig};

use crate::service::ServiceWorker;

#[derive(Parser)]
pub struct Cmd {
    // Path to the TON global config.
    #[clap(long)]
    pub global_config: PathBuf,

    // Path to the TON global config.
    #[clap(long)]
    pub service_config: PathBuf,
}

impl Cmd {
    pub async fn run(self) -> Result<()> {
        let global_config = TonGlobalConfig::load_from_file(self.global_config)?;
        let ton_lite_client =
            LiteClient::new(LiteClientConfig::default(), global_config.liteservers);

        let service_config = ServiceConfig::load_from_file(self.service_config)?;

        for worker_type in service_config.workers {
            match worker_type {
                WorkerType::L2Ton(config) => {
                    let l2_client = JrpcClient::new(config.l2_rcp_url.parse()?)?;
                    let worker =
                        ServiceWorker::new(l2_client, ton_lite_client.clone(), config).await?;

                    let _handle = tokio::spawn(async move {
                        tracing::info!("worker L2->TON started");
                        if let Err(e) = worker.run().await {
                            tracing::error!(%e, "worker L2->TON failed");
                        }
                        tracing::info!("worker L2->TON finished");
                    });
                }
                WorkerType::TonL2(config) => {
                    let l2_client = JrpcClient::new(config.l2_rcp_url.parse()?)?;
                    let worker =
                        ServiceWorker::new(ton_lite_client.clone(), l2_client, config).await?;

                    let _handle = tokio::spawn(async move {
                        tracing::info!("worker TON->L2 started");
                        if let Err(e) = worker.run().await {
                            tracing::error!(%e, "worker TON->L2 failed");
                        }
                        tracing::info!("worker TON->L2 finished");
                    });
                }
                WorkerType::L2L2(config) => {
                    let left_client = JrpcClient::new(config.left_rcp_url.as_str().parse()?)?;
                    let right_client = JrpcClient::new(config.right_rcp_url.as_str().parse()?)?;

                    let worker = ServiceWorker::new(left_client, right_client, config).await?;

                    let _handle = tokio::spawn(async move {
                        tracing::info!("worker L2->L2 started");
                        if let Err(e) = worker.run().await {
                            tracing::error!(%e, "worker L2->L2 failed");
                        }
                        tracing::info!("worker L2->L2 finished");
                    });
                }
            };
        }

        futures_util::future::pending::<()>().await;

        Ok(())
    }
}
