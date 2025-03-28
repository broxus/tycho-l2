use anyhow::Result;
use sync_service::config::WorkerConfigExt;
use sync_service::provider::{KeyBlockProvider, KeyBlockProviderClient};
use sync_service::uploader::{KeyBlockUploader, KeyBlockUploaderClient};

pub struct ServiceWorker<T1, T2> {
    provider: KeyBlockProvider<T1>,
    uploader: KeyBlockUploader<T2>,
}

impl<T1: KeyBlockProviderClient, T2: KeyBlockUploaderClient> ServiceWorker<T1, T2> {
    pub async fn new<C: WorkerConfigExt>(
        left_client: T1,
        right_client: T2,
        config: C,
    ) -> Result<Self> {
        let provider = KeyBlockProvider::new(left_client, config.block_provider()).await?;
        let uploader = KeyBlockUploader::new(right_client).await?;
        Ok(Self { provider, uploader })
    }

    pub async fn run(&self) -> Result<()> {
        while let Some(block) = self.provider.next_block().await {
            // TODO: upload v_set/signature
        }

        Ok(())
    }
}
