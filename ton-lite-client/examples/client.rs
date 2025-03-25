use std::net::SocketAddrV4;
use std::str::FromStr;

use anyhow::Result;
use everscale_types::cell::HashBytes;
use proof_api_util::block::{BlockchainBlock, BlockchainModels, TonModels};
use ton_lite_client::{LiteClient, LiteClientConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let server_pubkey = "n4VDnSCUuSpjnCyUk9e3QOOd6o0ItSWYbTnW3Wnn8wk=".parse::<HashBytes>()?;
    let server_address = SocketAddrV4::from_str("5.9.10.47:19949")?;

    let config = LiteClientConfig::from_addr_and_keys(server_address, server_pubkey);
    let client = LiteClient::new(&config).await?;

    // Get last key block proof
    {
        // Get mc info
        let mc_block_id = client.get_last_mc_block_id().await?;
        tracing::info!(?mc_block_id);

        // Get last mc block
        let mc_block = client
            .get_block(&mc_block_id)
            .await?
            .parse::<<TonModels as BlockchainModels>::Block>()?;

        let prev_key_block_seqno = mc_block.load_info()?.prev_key_block_seqno;
        tracing::info!(prev_key_block_seqno);

        // Last key block id
        let key_block_short_id = everscale_types::models::BlockIdShort {
            shard: mc_block_id.shard,
            seqno: prev_key_block_seqno,
        };
        let key_block_id = client.lookup_block(key_block_short_id).await?;
        tracing::info!(?key_block_id);

        // Block proof
        let proof = client.get_block_proof(&key_block_id).await?;
        tracing::info!(?proof);
    }

    Ok(())
}
