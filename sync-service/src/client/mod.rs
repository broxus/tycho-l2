use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use everscale_types::cell::Lazy;
use everscale_types::models::{
    BlockId, BlockSignature, BlockchainConfig, StdAddr, Transaction, ValidatorSet,
};
use everscale_types::prelude::*;
use serde::Deserialize;

pub use self::ton::TonClient;
pub use self::tycho::TychoClient;
use crate::util::account::{AccountStateResponse, LastTransactionId};

mod ton;
mod tycho;

#[async_trait]
pub trait NetworkClient: Send + Sync {
    fn name(&self) -> &str;

    async fn get_signature_id(&self) -> Result<Option<i32>>;

    async fn get_latest_key_block_seqno(&self) -> Result<u32>;

    async fn get_blockchain_config(&self) -> Result<BlockchainConfig>;

    async fn get_key_block(&self, seqno: u32) -> Result<KeyBlockData>;

    async fn get_account_state(
        &self,
        account: &StdAddr,
        last_transaction_lt: Option<u64>,
    ) -> Result<AccountStateResponse>;

    async fn get_transactions(
        &self,
        account: &StdAddr,
        lt: u64,
        hash: &HashBytes,
        count: u8,
    ) -> Result<Vec<Lazy<Transaction>>>;

    async fn send_message(&self, message: Cell) -> Result<()>;

    fn make_key_block_proof_to_sync(&self, data: &KeyBlockData) -> Result<Cell>;
}

impl dyn NetworkClient {
    pub async fn send_message_reliable(
        &self,
        address: &StdAddr,
        msg: Cell,
        known_lt: u64,
        expire_at: u32,
    ) -> Result<Lazy<Transaction>> {
        let msg_hash = *msg.repr_hash();

        self.send_message(msg)
            .await
            .context("failed to send message")?;

        self.find_transaction(address, &msg_hash, known_lt, Some(expire_at))
            .await
            .context("message expired")
    }

    pub async fn wait_for_deploy(&self, address: &StdAddr) {
        const POLL_INTERVAL: Duration = Duration::from_secs(1);

        loop {
            let state = self.get_account_state_with_retries(address, None).await;
            if matches!(state, AccountStateResponse::Exists { .. }) {
                break;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    pub async fn get_account_state_with_retries(
        &self,
        address: &StdAddr,
        known_lt: Option<u64>,
    ) -> AccountStateResponse {
        const RETRY_INTERVAL: Duration = Duration::from_secs(1);

        loop {
            match self.get_account_state(address, known_lt).await {
                Ok(res) => break res,
                Err(e) => {
                    tracing::warn!(client = self.name(), "failed to get contract state: {e:?}");
                    tokio::time::sleep(RETRY_INTERVAL).await;
                }
            }
        }
    }

    pub async fn find_transaction(
        &self,
        address: &StdAddr,
        msg_hash: &HashBytes,
        mut known_lt: u64,
        expire_at: Option<u32>,
    ) -> Option<Lazy<Transaction>> {
        const POLL_INTERVAL: Duration = Duration::from_secs(1);
        const RETRY_INTERVAL: Duration = Duration::from_secs(1);
        const BATCH_LEN: u8 = 10;

        let get_state =
            |known_lt: u64| self.get_account_state_with_retries(address, Some(known_lt));

        let do_find_transaction = async |mut last: LastTransactionId, known_lt: u64| loop {
            tracing::trace!(%address, ?last, known_lt, "fetching transactions");
            let res = self
                .get_transactions(address, last.lt, &last.hash, BATCH_LEN)
                .await?;
            anyhow::ensure!(!res.is_empty(), "got empty transactions response");

            for raw_tx in res {
                let hash = raw_tx.repr_hash();
                anyhow::ensure!(*hash == last.hash, "last tx hash mismatch");
                let tx = raw_tx.load().context("got invalid transaction")?;
                anyhow::ensure!(tx.lt == last.lt, "last tx lt mismatch");

                if let Some(in_msg) = &tx.in_msg {
                    if in_msg.repr_hash() == msg_hash {
                        return Ok(Some(raw_tx));
                    }
                }

                last = LastTransactionId {
                    lt: tx.prev_trans_lt,
                    hash: tx.prev_trans_hash,
                };
                if tx.prev_trans_lt <= known_lt {
                    break;
                }
            }

            if last.lt <= known_lt {
                return Ok(None);
            }
        };
        let find_transaction = async |last: LastTransactionId, known_lt: u64| loop {
            match do_find_transaction(last, known_lt).await {
                Ok(res) => break res,
                Err(e) => {
                    tracing::warn!(
                        client = self.name(),
                        "failed to process transactions: {e:?}",
                    );
                    tokio::time::sleep(RETRY_INTERVAL).await;
                }
            }
        };

        loop {
            let timings = match get_state(known_lt).await {
                AccountStateResponse::Exists {
                    timings,
                    last_transaction_id,
                    ..
                } => {
                    if last_transaction_id.lt > known_lt {
                        let res = find_transaction(last_transaction_id, known_lt).await;
                        if res.is_some() {
                            return res;
                        }

                        known_lt = last_transaction_id.lt;
                        tracing::trace!(%address, known_lt, "got new known lt");
                    }

                    timings
                }
                AccountStateResponse::NotExists { timings }
                | AccountStateResponse::Unchanged { timings } => timings,
            };

            // Message expired.
            if let Some(expire_at) = expire_at {
                if timings.gen_utime > expire_at {
                    return None;
                }
            }

            tracing::trace!(known_lt, %msg_hash, ?expire_at, "poll account");
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

#[derive(Debug)]
pub struct KeyBlockData {
    pub block_id: BlockId,
    pub root: Cell,
    pub prev_key_block_seqno: u32,
    pub current_vset: ValidatorSet,
    pub prev_vset: Option<ValidatorSet>,
    pub signatures: Vec<BlockSignature>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ClientConfig {
    Ton(TonClientConfig),
    Tycho(TychoClientConfig),
}

impl ClientConfig {
    pub fn build_client(&self) -> Result<Arc<dyn NetworkClient>> {
        use ton_lite_client::{LiteClient, TonGlobalConfig};

        use crate::util::jrpc_client::JrpcClient;

        Ok(match self {
            Self::Ton(config) => {
                let global_config = TonGlobalConfig::load_from_file(&config.global_config)
                    .with_context(|| format!("failed to load global config for {}", config.name))?;
                let rpc = LiteClient::new(Default::default(), global_config.liteservers);

                Arc::new(TonClient::new(config.name.clone(), rpc))
            }
            Self::Tycho(config) => {
                let rpc = JrpcClient::new(&config.rpc)
                    .with_context(|| format!("failed to create rpc client for {}", config.name))?;

                Arc::new(TychoClient::new(config.name.clone(), rpc))
            }
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TonClientConfig {
    /// Network name.
    pub name: String,
    /// Path to the global config.
    pub global_config: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TychoClientConfig {
    /// Network name.
    pub name: String,
    /// RPC URL.
    pub rpc: String,
}
