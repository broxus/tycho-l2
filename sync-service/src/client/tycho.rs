use anyhow::{Context, Result};
use async_trait::async_trait;
use everscale_types::cell::Lazy;
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{
    BlockSignatures, BlockchainConfig, GlobalCapability, StdAddr, Transaction,
};
use everscale_types::prelude::*;
use proof_api_util::block::{
    make_key_block_proof, BaseBlockProof, BlockchainBlock, BlockchainBlockExtra,
    BlockchainBlockMcExtra, BlockchainModels, TychoModels,
};

use crate::client::{KeyBlockData, NetworkClient};
use crate::util::account::AccountStateResponse;
use crate::util::jrpc_client::JrpcClient;

pub struct TychoClient {
    name: String,
    rpc: JrpcClient,
}

impl TychoClient {
    pub fn new(name: impl Into<String>, rpc: JrpcClient) -> Self {
        Self {
            name: name.into(),
            rpc,
        }
    }
}

#[async_trait]
impl NetworkClient for TychoClient {
    fn name(&self) -> &str {
        &self.name
    }

    async fn get_signature_id(&self) -> Result<Option<i32>> {
        let current = self.rpc.get_latest_config().await?;
        let global = current.config.get_global_version()?;
        Ok(global
            .capabilities
            .contains(GlobalCapability::CapSignatureWithId)
            .then_some(current.global_id))
    }

    async fn get_latest_key_block_seqno(&self) -> Result<u32> {
        Ok(self.rpc.get_latest_config().await?.seqno)
    }

    async fn get_blockchain_config(&self) -> Result<BlockchainConfig> {
        Ok(self.rpc.get_latest_config().await?.config)
    }

    async fn get_key_block(&self, seqno: u32) -> Result<KeyBlockData> {
        let res = self.rpc.get_key_block_proof(seqno).await?;
        let Some(proof) = res.proof else {
            anyhow::bail!("key block not found");
        };
        let block_id = res.block_id.context("expected block id in rpc response")?;
        let proof = BocRepr::decode_base64::<BaseBlockProof<BlockSignatures>, _>(proof)
            .context("failed to deserialize key block proof")?;

        // TODO: Check signatures.
        let signatures = match proof.signatures {
            Some(data) => {
                let mut signatures = Vec::new();
                for item in data.load()?.signatures.values() {
                    signatures.push(item?);
                }
                signatures
            }
            None => anyhow::bail!("masterchain block proof doesn't contain signatures"),
        };

        let root = proof.root.parse_exotic::<MerkleProof>()?.cell;
        let block = root.parse::<<TychoModels as BlockchainModels>::Block>()?;

        let prev_key_block_seqno = block.load_info()?.prev_key_block_seqno;

        let custom = block
            .load_extra()?
            .load_custom()?
            .context("expected McBlockCustom")?;
        let config = custom.config().context("expected config")?;

        Ok(KeyBlockData {
            block_id,
            root,
            prev_key_block_seqno,
            signatures,
            current_vset: config.get_current_validator_set()?,
            prev_vset: config.get_previous_validator_set()?,
        })
    }

    async fn get_account_state(
        &self,
        account: &StdAddr,
        last_transaction_lt: Option<u64>,
    ) -> Result<AccountStateResponse> {
        self.rpc
            .get_account_state(account, last_transaction_lt)
            .await
    }

    async fn get_transactions(
        &self,
        account: &StdAddr,
        lt: u64,
        hash: &HashBytes,
        count: u8,
    ) -> Result<Vec<Lazy<Transaction>>> {
        let transactions = self.rpc.get_transactions(account, Some(lt), count).await?;
        let mut is_first = true;
        let mut result = Vec::with_capacity(transactions.len());
        for tx in transactions {
            let tx = Boc::decode_base64(tx).context("failed to deserialize transaction")?;
            if std::mem::take(&mut is_first) {
                anyhow::ensure!(tx.repr_hash() == hash, "latest tx hash mismatch");
            }

            result.push(Lazy::from_raw(tx)?);
        }
        Ok(result)
    }

    async fn send_message(&self, message: Cell) -> Result<()> {
        self.rpc.send_message(message.as_ref()).await
    }

    fn make_key_block_proof_to_sync(&self, data: &KeyBlockData) -> Result<Cell> {
        make_key_block_proof::<TychoModels>(
            data.root.clone(),
            data.prev_vset
                .as_ref()
                .map(|prev_vset| data.current_vset.utime_since != prev_vset.utime_until)
                .unwrap_or_default(),
        )
        .context("failed to build key block proof")
    }
}
