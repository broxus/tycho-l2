use anyhow::Result;
use sync_service::stream::{BlockStream, BlockStreamConfig};
use sync_service::utils::jrpc_client::JrpcClient;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let url = reqwest::Url::parse("https://rpc-devnet1.tychoprotocol.com/")?;
    let client = JrpcClient::new(url)?;

    let block_stream_config: BlockStreamConfig =
        serde_json::from_str(include_str!("block_stream.json"))?;

    let stream = BlockStream::new(client, block_stream_config).await?;
    while let Some(block) = stream.next_block().await {
        tracing::info!(utime_since = block.v_set.utime_since);
    }

    Ok(())
}
