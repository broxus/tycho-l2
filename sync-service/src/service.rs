use anyhow::Result;
use sync_service::config::WorkerConfigExt;
use sync_service::provider::{BlockProvider, BlockProviderClient};

pub struct ServiceWorker<T> {
    block_provider: BlockProvider<T>,
}

impl<T: BlockProviderClient> ServiceWorker<T> {
    pub async fn new<C: WorkerConfigExt>(client: T, config: C) -> Result<Self> {
        let block_provider = BlockProvider::new(client, config.block_provider()).await?;
        Ok(Self { block_provider })
    }

    pub async fn run(&self) -> Result<()> {
        while let Some(block) = self.block_provider.next_block().await {
            // TODO: deploy v_set/signature from block
        }

        Ok(())
    }
}
