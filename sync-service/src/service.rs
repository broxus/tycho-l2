use anyhow::Result;
use sync_service::config::WorkerConfigExt;
use sync_service::stream::{BlockStream, BlockStreamClient};

pub struct ServiceWorker<T> {
    block_stream: BlockStream<T>,
}

impl<T: BlockStreamClient> ServiceWorker<T> {
    pub async fn new<C: WorkerConfigExt>(block_stream_client: T, config: C) -> Result<Self> {
        let block_stream = BlockStream::new(block_stream_client, config.block_stream()).await?;
        Ok(Self { block_stream })
    }

    pub async fn run(&self) -> Result<()> {
        while let Some(block) = self.block_stream.next_block().await {
            // TODO: deploy v_set/signature from block
        }

        Ok(())
    }
}
