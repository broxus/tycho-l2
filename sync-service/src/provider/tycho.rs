use anyhow::{Context, Result};
use async_trait::async_trait;
use everscale_types::boc::{Boc, BocRepr};
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{BlockSignatures, BlockchainConfig, OptionalAccount, StdAddr};
use everscale_types::prelude::Load;
use proof_api_util::block::{BaseBlockProof, BlockchainBlock, BlockchainModels, TychoModels};

use crate::provider::{KeyBlockData, KeyBlockProviderClient};
use crate::utils::jrpc_client::{AccountStateResponse, JrpcClient};

#[async_trait]
impl KeyBlockProviderClient for JrpcClient {
    async fn get_last_key_block(&self) -> Result<KeyBlockData> {
        let res = self.get_latest_key_block().await?;

        let cell = Boc::decode_base64(res.block)?;
        let latest_key_block = cell.parse::<<TychoModels as BlockchainModels>::Block>()?;

        let seqno = latest_key_block.load_info()?.seqno;

        self.get_key_block(seqno).await
    }

    async fn get_key_block(&self, seqno: u32) -> Result<KeyBlockData> {
        let res = self.get_key_block_proof(seqno).await?;
        let proof = BocRepr::decode_base64::<BaseBlockProof<BlockSignatures>, _>(
            res.proof.ok_or(TychoBlockProviderError::ProofNotFound)?,
        )?;

        let signatures = proof
            .signatures
            .ok_or(TychoBlockProviderError::SignaturesNotFound)?
            .load()?
            .signatures
            .iter()
            .map(|x| Ok(x?.1))
            .collect::<Result<Vec<_>>>()?;

        let cell = proof.root.parse_exotic::<MerkleProof>()?.cell;
        let block = cell.parse::<<TychoModels as BlockchainModels>::Block>()?;

        let prev_seqno = block.load_info()?.prev_key_block_seqno;

        let custom = block.load_extra()?.custom.context("key block not full")?;

        let mut slice = custom.as_slice()?;
        slice.only_last(256, 1)?;

        let blockchain_config = BlockchainConfig::load_from(&mut slice)?;

        let v_set = blockchain_config.get_current_validator_set()?;

        Ok(KeyBlockData {
            prev_seqno,
            v_set,
            signatures,
        })
    }

    async fn get_blockchain_config(&self) -> Result<BlockchainConfig> {
        let config = self.get_config().await?;
        Ok(config.config)
    }

    async fn get_account_state(&self, account: StdAddr) -> Result<OptionalAccount> {
        let state = self.get_account(&account).await?;
        match state {
            AccountStateResponse::Exists { account, .. } => Ok(OptionalAccount(Some(*account))),
            AccountStateResponse::Unchanged { .. } | AccountStateResponse::NotExists { .. } => {
                Ok(OptionalAccount::EMPTY)
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum TychoBlockProviderError {
    #[error("signatures not found in key block")]
    SignaturesNotFound,
    #[error("proof not found")]
    ProofNotFound,
}
