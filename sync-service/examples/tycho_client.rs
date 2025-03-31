use anyhow::{Context, Result};
use everscale_types::boc::{Boc, BocRepr};
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{BlockSignatures, BlockchainConfig};
use everscale_types::prelude::Load;
use proof_api_util::block::{
    BaseBlockProof, BlockchainBlock, BlockchainBlockExtra, BlockchainBlockMcExtra,
    BlockchainModels, TychoModels,
};
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

        let cell = Boc::decode_base64(key_block.block)?;

        let key_block = cell.parse::<<TychoModels as BlockchainModels>::Block>()?;

        let key_block_seqno = key_block.load_info()?.seqno;
        tracing::info!(key_block_seqno);

        // Proof
        let res = client.get_key_block_proof(key_block_seqno).await?;
        let proof = BocRepr::decode_base64::<BaseBlockProof<BlockSignatures>, _>(
            res.proof.context("proof not found")?,
        )?;

        let block = proof.root.parse_exotic::<MerkleProof>()?.cell;

        let block = block.parse::<<TychoModels as BlockchainModels>::Block>()?;

        let custom = block
            .load_extra()?
            .load_custom()?
            .context("key block not full")?;

        let blockchain_config = custom.config().context("expected key block")?;

        let v_set = blockchain_config.get_current_validator_set()?;
        tracing::info!(?v_set);
    }

    Ok(())
}
