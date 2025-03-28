use anyhow::Result;
use sync_service::provider::{BlockProviderConfig, KeyBlockProvider};
use sync_service::utils::jrpc_client::JrpcClient;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let url = reqwest::Url::parse("https://rpc-devnet1.tychoprotocol.com/")?;
    let client = JrpcClient::new(url)?;

    let block_stream_config: BlockProviderConfig =
        serde_json::from_str(include_str!("block_provider.json"))?;

    let stream = KeyBlockProvider::new(client, block_stream_config).await?;
    while let Some(block) = stream.next_block().await {
        tracing::info!(utime_since = block.v_set.utime_since);
    }

    Ok(())
}
