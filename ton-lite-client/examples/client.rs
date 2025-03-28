use std::str::FromStr;

use anyhow::Result;
use everscale_types::boc::Boc;
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{BlockchainConfig, OptionalAccount, StdAddr};
use proof_api_util::block::{BlockchainBlock, BlockchainModels, TonModels};
use ton_lite_client::{LiteClient, LiteClientConfig, TonGlobalConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let global_config: TonGlobalConfig =
        serde_json::from_str(include_str!("ton-global-config.json"))?;

    let config = LiteClientConfig::default();
    let client = LiteClient::new(config, global_config.liteservers);

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

        // // Block proof
        // let proof = client.get_block_proof(&key_block_id).await?;
        // tracing::info!(?proof);

        // let proof = everscale_types::boc::Boc::decode(&proof.config_proof)?
        //     .parse_exotic::<MerkleProof>()?;

        // let block = proof
        //     .cell
        //     .parse::<<TonModels as BlockchainModels>::Block>()?;

        // if let Some(custom) = block.load_extra()?.custom {
        //     let mut slice = custom.as_slice()?;
        //     slice.only_last(256, 1)?;

        //     let config = BlockchainConfig::load_from(&mut slice)?;
        //     let v_set = config.get_current_validator_set()?;
        //     tracing::info!(?v_set);
        // }
    }

    // Get blockchain config
    {
        let mc_block_id = client.get_last_mc_block_id().await?;
        tracing::info!(?mc_block_id);

        let config = client.get_config(&mc_block_id).await?;

        let proof = Boc::decode(&config.config_proof)?.parse_exotic::<MerkleProof>()?;

        let config = proof.cell.parse::<BlockchainConfig>()?;
        tracing::info!(?config);
    }

    // Get account state
    {
        // Get mc info
        let mc_block_id = client.get_last_mc_block_id().await?;
        tracing::info!(?mc_block_id);

        let addr = StdAddr::from_str(
            "0:69884128d07de140f313e1238557261f4e5f849315df3eadc7b56961356bdf61",
        )?;
        let state = client.get_account(mc_block_id, addr).await?;
        let cell = Boc::decode(&state.state)?;

        let account = cell.parse::<OptionalAccount>()?;
        tracing::info!(?account);
    }

    Ok(())
}
