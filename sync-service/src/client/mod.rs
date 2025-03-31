use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use everscale_types::cell::Cell;
use everscale_types::models::{BlockId, BlockSignature, BlockchainConfig, StdAddr, ValidatorSet};
use serde::Deserialize;

pub use self::ton::TonClient;
pub use self::tycho::TychoClient;
use crate::util::account::AccountStateResponse;

mod ton;
mod tycho;

#[async_trait]
pub trait NetworkClient: Send + Sync {
    fn name(&self) -> &str;

    async fn get_latest_key_block_seqno(&self) -> Result<u32>;

    async fn get_blockchain_config(&self) -> Result<BlockchainConfig>;

    async fn get_key_block(&self, seqno: u32) -> Result<KeyBlockData>;

    async fn get_account_state(
        &self,
        account: &StdAddr,
        last_transaction_lt: Option<u64>,
    ) -> Result<AccountStateResponse>;
}

#[async_trait]
impl<T: NetworkClient> NetworkClient for Box<T> {
    fn name(&self) -> &str {
        T::name(&*self)
    }

    async fn get_latest_key_block_seqno(&self) -> Result<u32> {
        T::get_latest_key_block_seqno(&*self).await
    }

    async fn get_blockchain_config(&self) -> Result<BlockchainConfig> {
        T::get_blockchain_config(&*self).await
    }

    async fn get_key_block(&self, seqno: u32) -> Result<KeyBlockData> {
        T::get_key_block(&*self, seqno).await
    }

    async fn get_account_state(
        &self,
        account: &StdAddr,
        last_transaction_lt: Option<u64>,
    ) -> Result<AccountStateResponse> {
        T::get_account_state(&*self, account, last_transaction_lt).await
    }
}

#[async_trait]
impl<T: NetworkClient> NetworkClient for Arc<T> {
    fn name(&self) -> &str {
        T::name(&*self)
    }

    async fn get_latest_key_block_seqno(&self) -> Result<u32> {
        T::get_latest_key_block_seqno(&*self).await
    }

    async fn get_blockchain_config(&self) -> Result<BlockchainConfig> {
        T::get_blockchain_config(&*self).await
    }

    async fn get_key_block(&self, seqno: u32) -> Result<KeyBlockData> {
        T::get_key_block(&*self, seqno).await
    }

    async fn get_account_state(
        &self,
        account: &StdAddr,
        last_transaction_lt: Option<u64>,
    ) -> Result<AccountStateResponse> {
        T::get_account_state(&*self, account, last_transaction_lt).await
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
