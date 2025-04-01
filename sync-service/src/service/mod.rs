use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use everscale_crypto::ed25519;
use everscale_types::cell::{CellBuilder, HashBytes};
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{Account, AccountState, BlockchainConfig, StdAddr};
use everscale_types::num::Tokens;
use nekoton_abi::execution_context::ExecutionContextBuilder;
use num_traits::ToPrimitive;
use proof_api_util::block::prepare_signatures;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tycho_util::serde_helpers;

use self::wallet::Wallet;
use crate::client::{KeyBlockData, NetworkClient};
use crate::util::account::AccountStateResponse;

mod lib_store;
mod wallet;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UploaderConfig {
    #[serde(with = "proof_api_util::serde_helpers::ton_address")]
    pub bridge_address: StdAddr,

    #[serde(with = "proof_api_util::serde_helpers::ton_address")]
    pub wallet_address: StdAddr,
    pub wallet_secret: HashBytes,

    #[serde(with = "serde_helpers::string")]
    pub lib_store_value: u128,
    #[serde(with = "serde_helpers::string")]
    pub store_vset_value: u128,

    #[serde(default = "default_poll_interval")]
    pub poll_interval: Duration,

    #[serde(default = "default_retry_interval", with = "serde_helpers::humantime")]
    pub retry_interval: Duration,
}

fn default_poll_interval() -> Duration {
    Duration::from_secs(2)
}

fn default_retry_interval() -> Duration {
    Duration::from_secs(1)
}

pub struct Uploader {
    src: Arc<dyn NetworkClient>,
    dst: Arc<dyn NetworkClient>,
    config: UploaderConfig,
    /// Blockchain config of the `dst` network.
    blockchain_config: BlockchainConfig,
    /// Cache of key blocks from the `src` network.
    key_blocks_cache: BTreeMap<u32, Arc<KeyBlockData>>,
    wallet: Wallet,
}

impl Uploader {
    pub async fn new(
        src: Arc<dyn NetworkClient>,
        dst: Arc<dyn NetworkClient>,
        config: UploaderConfig,
    ) -> Result<Self> {
        let blockchain_config = dst
            .get_blockchain_config()
            .await
            .with_context(|| format!("failed to get blockchain config for {}", dst.name()))?;

        let secret = ed25519::SecretKey::from_bytes(config.wallet_secret.0);
        let keypair = Arc::new(ed25519::KeyPair::from(&secret));
        let wallet = Wallet::new(config.wallet_address.workchain, keypair, dst.clone());
        anyhow::ensure!(
            *wallet.address() == config.wallet_address,
            "wallet address mismatch for {}: expected={}, got={}",
            dst.name(),
            config.wallet_address,
            wallet.address(),
        );

        Ok(Self {
            src,
            dst,
            config,
            blockchain_config,
            key_blocks_cache: Default::default(),
            wallet,
        })
    }

    #[tracing::instrument(name = "uploader", skip_all, fields(
        src = self.src.name(),
        dst = self.dst.name(),
    ))]
    pub async fn run(mut self) {
        loop {
            if let Err(e) = self.sync_key_blocks().await {
                tracing::error!("failed to sync key blocks: {e:?}");
            }
            tokio::time::sleep(self.config.poll_interval).await;
        }
    }

    pub async fn sync_key_blocks(&mut self) -> Result<()> {
        let current_vset_utime_since = self
            .get_current_epoch_since()
            .await
            .context("failed to get current epoch")?;
        tracing::info!(current_vset_utime_since);

        let Some(key_block) = self.find_next_key_block(current_vset_utime_since).await? else {
            tracing::debug!(current_vset_utime_since, "no new key blocks found");
            return Ok(());
        };

        tracing::info!(block_id = %key_block.block_id, "sending key block");
        self.send_key_block(key_block.clone())
            .await
            .context("failed to send key block")?;

        let new_cache = self
            .key_blocks_cache
            .split_off(&key_block.prev_key_block_seqno);
        self.key_blocks_cache = new_cache;
        tracing::debug!(key_blocks_cache_len = self.key_blocks_cache.len());
        Ok(())
    }

    async fn send_key_block(&self, key_block: Arc<KeyBlockData>) -> Result<()> {
        let key_block_proof = self.src.make_key_block_proof_to_sync(&key_block)?;
        let key_block_proof = CellBuilder::build_from(MerkleProof {
            hash: *key_block_proof.hash(0),
            depth: key_block_proof.depth(0),
            cell: key_block_proof,
        })?;

        let Some(prev_vset) = key_block.prev_vset.as_ref() else {
            anyhow::bail!("no prev_vset found");
        };
        let signatures =
            prepare_signatures(key_block.signatures.iter().cloned().map(Ok), &prev_vset)?;

        // Deploy library.
        let id = rand::thread_rng().gen();
        let lib_store = self
            .wallet
            .deploy_vset_lib(
                &key_block.current_vset,
                Tokens::new(self.config.lib_store_value),
                id,
            )
            .await
            .context("failed to deploy a library with validator set")?;
        tracing::info!(
            seqno = %key_block.block_id.seqno,
            address = %lib_store,
            "deployed a lib_store for key block",
        );

        // Send key block.
        self.wallet
            .send_key_block(
                key_block_proof,
                &key_block.block_id.file_hash,
                signatures,
                &self.config.bridge_address,
                Tokens::new(self.config.store_vset_value),
                0,
            )
            .await
            .context("failed to store key block proof into bridge contract")?;

        // Done
        Ok(())
    }

    async fn find_next_key_block(
        &mut self,
        current_vset_utime_since: u32,
    ) -> Result<Option<Arc<KeyBlockData>>> {
        // TODO: Add retries.
        let mut latest_seqno = self.src.get_latest_key_block_seqno().await?;

        let mut result = None;
        loop {
            let key_block = self.get_key_block(latest_seqno).await?;

            let vset_utime_since = key_block.current_vset.utime_since;
            match vset_utime_since.cmp(&current_vset_utime_since) {
                // Skip and remember all key blocks newer than the current vset.
                std::cmp::Ordering::Greater => {
                    latest_seqno = key_block.prev_key_block_seqno;
                    result = Some(key_block);
                }
                // Handle the case when the rpc is out of sync.
                std::cmp::Ordering::Less => {
                    tracing::warn!(
                        seqno = latest_seqno,
                        vset_utime_since,
                        "the latest key block has too old vset"
                    );
                    return Ok(None);
                }
                // Stop on the same vset.
                std::cmp::Ordering::Equal => break Ok(result),
            }
        }
    }

    async fn get_key_block(&mut self, seqno: u32) -> Result<Arc<KeyBlockData>> {
        if let Some(key_block) = self.key_blocks_cache.get(&seqno) {
            return Ok(key_block.clone());
        }

        // TODO: Add retries.
        let key_block = self.src.get_key_block(seqno).await.map(Arc::new)?;
        self.key_blocks_cache.insert(seqno, key_block.clone());

        tracing::debug!(
            seqno,
            vset_utime_since = key_block.current_vset.utime_since,
            "found new key block"
        );
        Ok(key_block)
    }

    async fn get_current_epoch_since(&self) -> Result<u32> {
        // TODO: Add retries.
        let account = self.get_bridge_account().await?;

        let context = ExecutionContextBuilder::new(&account)
            .with_config(self.blockchain_config.clone())
            .build()
            .context("build executor failed")?;

        let result = context
            .run_getter("get_state_short", &[])
            .context("run_getter failed")?;
        anyhow::ensure!(
            result.success,
            "failed to get current epoch, exit_code={}",
            result.exit_code
        );

        let get_utime_since = move || {
            let first = result.stack.into_iter().next().context("empty stack")?;
            let int = first.into_int()?;
            int.to_u32().context("int out of range")
        };

        get_utime_since().context("invalid getter output")
    }

    async fn get_bridge_account(&self) -> Result<Box<Account>> {
        match self
            .dst
            .get_account_state(&self.config.bridge_address, None)
            .await
            .context("failed to get bridge account")?
        {
            AccountStateResponse::Exists { account, .. } => {
                anyhow::ensure!(
                    matches!(&account.state, AccountState::Active(..)),
                    "bridge account is not active"
                );
                Ok(account)
            }
            AccountStateResponse::Unchanged { .. } => anyhow::bail!("unexpected response"),
            AccountStateResponse::NotExists { .. } => anyhow::bail!("bridge account not found"),
        }
    }
}
