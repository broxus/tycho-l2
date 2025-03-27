use anyhow::Result;
use async_trait::async_trait;
use everscale_types::boc::Boc;
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{
    BlockIdShort, BlockchainConfig, OptionalAccount, ShardIdent, StdAddr,
};
use everscale_types::prelude::Load;
use proof_api_util::block::{BlockchainBlock, BlockchainModels, TonModels};
use ton_lite_client::{proto, LiteClient};

use crate::stream::{BlockchainClient, KeyBlockData};

#[async_trait]
impl BlockchainClient for LiteClient {
    async fn get_last_key_block(&self) -> Result<KeyBlockData> {
        let mc_block_id = self.get_last_mc_block_id().await?;

        let mc_block = self
            .get_block(&mc_block_id)
            .await?
            .parse::<<TonModels as BlockchainModels>::Block>()?;

        let prev_key_block_seqno = mc_block.load_info()?.prev_key_block_seqno;

        self.get_key_block(prev_key_block_seqno).await
    }

    async fn get_key_block(&self, seqno: u32) -> Result<KeyBlockData> {
        let key_block_short_id = BlockIdShort {
            shard: ShardIdent::MASTERCHAIN,
            seqno,
        };

        let key_block_id = self.lookup_block(key_block_short_id).await?;

        // TODO: Check signatures.
        let key_block_proof = 'proof: {
            let partial = self.get_block_proof(&key_block_id, None).await?;
            for step in partial.steps {
                if let proto::BlockLink::BlockLinkForward(proof) = step {
                    break 'proof proof;
                }
            }

            anyhow::bail!("proof not found");
        };

        let proof = Boc::decode(&key_block_proof.config_proof)?.parse_exotic::<MerkleProof>()?;

        let block = proof
            .cell
            .parse::<<TonModels as BlockchainModels>::Block>()?;

        let prev_seqno = block.load_info()?.prev_key_block_seqno;

        let custom = block
            .load_extra()?
            .custom
            .ok_or(TonBlockStreamError::KeyBlockNotFull)?;

        let mut slice = custom.as_slice()?;
        slice.only_last(256, 1)?;

        let blockchain_config = BlockchainConfig::load_from(&mut slice)?;
        let v_set = blockchain_config.get_current_validator_set()?;

        let signatures = key_block_proof.signatures.signatures;

        Ok(KeyBlockData {
            prev_seqno,
            v_set,
            signatures,
        })
    }

    async fn get_blockchain_config(&self) -> Result<BlockchainConfig> {
        let mc_block_id = self.get_last_mc_block_id().await?;
        let config = self.get_config(&mc_block_id).await?;

        let config_proof = Boc::decode(&config.config_proof)?.parse_exotic::<MerkleProof>()?;
        let blockchain_config = config_proof.cell.parse::<BlockchainConfig>()?;

        Ok(blockchain_config)
    }

    async fn get_account_state(&self, account: StdAddr) -> Result<OptionalAccount> {
        let mc_block_id = self.get_last_mc_block_id().await?;
        let account_state = self.get_account(mc_block_id, account).await?;

        let cell = Boc::decode(&account_state.state)?;
        let account = cell.parse::<OptionalAccount>()?;

        Ok(account)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum TonBlockStreamError {
    #[error("key block not full")]
    KeyBlockNotFull,
    #[error("failed to convert signature")]
    InvalidSignatureLength,
}
