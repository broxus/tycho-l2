use anyhow::Result;
use sync_service::utils::jrpc_client::JrpcClient;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let url = reqwest::Url::parse("https://rpc-devnet1.tychoprotocol.com/")?;
    let client = JrpcClient::new(url)?;

    // Get last key block proof
    {
        // Last key block
        let key_block = client.get_latest_key_block().await?;
        let key_block_seqno = key_block.block.load_info()?.seqno;
        tracing::info!(key_block_seqno);

        // Proof
        let proof = client.get_key_block_proof(key_block_seqno).await?;
        tracing::info!(?proof);
    }

    Ok(())
}
