use async_trait::async_trait;
use ton_lite_client::LiteClient;

use crate::uploader::KeyBlockUploaderClient;

#[async_trait]
impl KeyBlockUploaderClient for LiteClient {
    async fn test(&self) -> anyhow::Result<()> {
        todo!()
    }
}
