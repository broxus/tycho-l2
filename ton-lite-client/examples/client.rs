use anyhow::Result;
use everscale_types::boc::Boc;
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{BlockIdShort, BlockchainConfig, OptionalAccount};
use proof_api_util::block::{
    check_signatures, BlockchainBlock, BlockchainBlockExtra, BlockchainBlockMcExtra,
    BlockchainModels, TonModels,
};
use ton_lite_client::{proto, LiteClient, LiteClientConfig, TonGlobalConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let global_config: TonGlobalConfig =
        serde_json::from_str(include_str!("ton-global-config.json"))?;

    let config = LiteClientConfig::default();
    let client = LiteClient::new(config, global_config.liteservers);

    // Check block proof
    {
        // Get mc info
        let mc_block_id = client.get_last_mc_block_id().await?;
        tracing::info!(?mc_block_id);

        // Get last mc block
        let mc_block = client
            .get_block(&mc_block_id)
            .await?
            .parse::<<TonModels as BlockchainModels>::Block>()?;

        // Last key block
        let (id, key_block) = {
            let seqno = mc_block.load_info()?.prev_key_block_seqno;

            let short_id = BlockIdShort {
                shard: mc_block_id.shard,
                seqno,
            };
            let id = client.lookup_block(short_id).await?;

            let block = client
                .get_block(&id)
                .await?
                .parse::<<TonModels as BlockchainModels>::Block>()?;

            (id, block)
        };

        // Prev key block
        let prev_id = {
            let seqno = key_block.load_info()?.prev_key_block_seqno;
            let short_id = BlockIdShort {
                shard: mc_block_id.shard,
                seqno,
            };

            client.lookup_block(short_id).await?
        };

        // Block proof
        let proof = client.get_block_proof(&prev_id, Some(&id), false).await?;
        let key_block_proof = 'proof: {
            for step in proof.steps {
                if let proto::BlockLink::BlockLinkForward(proof) = step {
                    break 'proof proof;
                }
            }

            anyhow::bail!("proof not found");
        };
        assert!(key_block_proof.to_key_block);

        let v_set = {
            let proof =
                Boc::decode(&key_block_proof.config_proof)?.parse_exotic::<MerkleProof>()?;

            let dest_proof =
                Boc::decode(&key_block_proof.dest_proof)?.parse_exotic::<MerkleProof>()?;

            assert_eq!(id.root_hash.0, dest_proof.cell.hash(0).0);

            let block = proof
                .cell
                .parse::<<TonModels as BlockchainModels>::Block>()?;

            let custom = block.load_extra()?.load_custom()?.expect("should exist");
            let config = custom.config().expect("expected key block");
            config.get_current_validator_set()?
        };

        let signatures = key_block_proof.signatures.signatures;

        check_signatures(&id, signatures.into_iter().map(Ok), &v_set)?;
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

        let addr = "0:69884128d07de140f313e1238557261f4e5f849315df3eadc7b56961356bdf61".parse()?;
        let state = client.get_account(&mc_block_id, &addr).await?;
        let cell = Boc::decode(&state.state)?;

        let account = cell.parse::<OptionalAccount>()?;
        tracing::info!(?account);
    }

    Ok(())
}
