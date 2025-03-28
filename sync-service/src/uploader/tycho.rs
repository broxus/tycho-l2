use async_trait::async_trait;

use crate::uploader::KeyBlockUploaderClient;
use crate::utils::jrpc_client::JrpcClient;

#[async_trait]
impl KeyBlockUploaderClient for JrpcClient {
    async fn test(&self) -> anyhow::Result<()> {
        todo!()
    }
}
