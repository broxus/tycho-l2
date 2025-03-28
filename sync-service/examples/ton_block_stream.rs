use anyhow::Result;
use sync_service::stream::{BlockStream, BlockStreamConfig};
use ton_lite_client::{LiteClient, LiteClientConfig, TonGlobalConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let global_config: TonGlobalConfig =
        serde_json::from_str(include_str!("ton-global-config.json"))?;

    let config = LiteClientConfig::default();
    let client = LiteClient::new(config, global_config.liteservers);

    let block_stream_config: BlockStreamConfig =
        serde_json::from_str(include_str!("block_stream.json"))?;

    let stream = BlockStream::new(client, block_stream_config).await?;
    while let Some(block) = stream.next_block().await {
        tracing::info!(prev_seqno = block.prev_seqno);
    }

    Ok(())
}
