use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use everscale_types::models::{BlockchainConfig, OptionalAccount, StdAddr};
use nekoton_abi::execution_context::ExecutionContextBuilder;
use parking_lot::Mutex;
use serde::Deserialize;
use tycho_util::serde_helpers;

pub mod ton;
pub mod tycho;

#[async_trait]
pub trait KeyBlockProviderClient: Send + Sync {
    async fn get_last_key_block(&self) -> anyhow::Result<KeyBlockData>;

    async fn get_key_block(&self, seqno: u32) -> anyhow::Result<KeyBlockData>;

    async fn get_blockchain_config(&self) -> anyhow::Result<BlockchainConfig>;

    async fn get_account_state(&self, account: StdAddr) -> anyhow::Result<OptionalAccount>;
}

#[async_trait]
impl KeyBlockProviderClient for Box<dyn KeyBlockProviderClient + Send + Sync> {
    async fn get_last_key_block(&self) -> anyhow::Result<KeyBlockData> {
        self.as_ref().get_last_key_block().await
    }

    async fn get_key_block(&self, seqno: u32) -> anyhow::Result<KeyBlockData> {
        self.as_ref().get_key_block(seqno).await
    }

    async fn get_blockchain_config(&self) -> anyhow::Result<BlockchainConfig> {
        self.as_ref().get_blockchain_config().await
    }

    async fn get_account_state(&self, account: StdAddr) -> anyhow::Result<OptionalAccount> {
        self.as_ref().get_account_state(account).await
    }
}

pub struct KeyBlockProvider<T> {
    client: T,
    config: BlockProviderConfig,
    blockchain_config: BlockchainConfig,
    cache: Mutex<BTreeMap<u32, KeyBlockData>>,
    last_known_utime_since: ArcSwapOption<u32>,
}

impl<T: KeyBlockProviderClient> KeyBlockProvider<T> {
    pub async fn new(client: T, config: BlockProviderConfig) -> anyhow::Result<Self> {
        let blockchain_config = client.get_blockchain_config().await?;

        Ok(Self {
            client,
            config,
            blockchain_config,
            cache: Default::default(),
            last_known_utime_since: Default::default(),
        })
    }

    pub async fn next_block(&self) -> Option<KeyBlockData> {
        let config = &self.config;

        {
            let mut cache = self.cache.lock();
            if !cache.is_empty() {
                let block_info = cache.pop_first().map(|(_, v)| v);

                // Update last_known_utime_since
                if let Some(block_info) = &block_info {
                    self.last_known_utime_since
                        .store(Some(Arc::new(block_info.v_set.utime_since)));
                }

                return block_info;
            }
        }

        'polling: loop {
            let last_known_utime_since = match self.last_known_utime_since.load_full() {
                Some(utime_since) => *utime_since,
                None => match self.get_current_epoch_since().await {
                    Ok(utime_since) => utime_since,
                    Err(e) => {
                        tracing::error!("failed to get last key block: {e}");
                        tokio::time::sleep(config.error_timeout).await;
                        continue 'polling;
                    }
                },
            };

            match self.client.get_last_key_block().await {
                Ok(block_info) if block_info.v_set.utime_since > last_known_utime_since => {
                    let mut prev_key_block_seqno = block_info.prev_seqno;

                    {
                        let mut cache = self.cache.lock();
                        cache.insert(block_info.v_set.utime_since, block_info);
                    }

                    'traversing: loop {
                        match self.client.get_key_block(prev_key_block_seqno).await {
                            Ok(block_info)
                                if block_info.v_set.utime_since > last_known_utime_since =>
                            {
                                prev_key_block_seqno = block_info.prev_seqno;

                                {
                                    let mut cache = self.cache.lock();
                                    cache.insert(block_info.v_set.utime_since, block_info);
                                }

                                continue 'traversing;
                            }
                            Ok(block_info)
                                if block_info.v_set.utime_since == last_known_utime_since =>
                            {
                                let block_info = self.cache.lock().pop_first().map(|(_, v)| v);

                                // Update last_known_utime_since
                                if let Some(block_info) = &block_info {
                                    self.last_known_utime_since
                                        .store(Some(Arc::new(block_info.v_set.utime_since)));
                                }

                                return block_info;
                            }
                            Err(e) => {
                                tracing::error!(
                                    seqno = prev_key_block_seqno,
                                    "failed to get key block: {e}",
                                );
                                tokio::time::sleep(config.error_timeout).await;
                                continue;
                            }
                            _ => return None, // Finish provider (shouldn't happen)
                        }
                    }
                }
                Ok(block_info) if block_info.v_set.utime_since == last_known_utime_since => {
                    tokio::time::sleep(config.polling_timeout).await;
                    continue 'polling;
                }
                Err(e) => {
                    tracing::error!("failed to get last key block: {e}");
                    tokio::time::sleep(config.error_timeout).await;
                    continue 'polling;
                }
                _ => return None, // Finish provider (shouldn't happen)
            }
        }
    }

    async fn get_current_epoch_since(&self) -> anyhow::Result<u32> {
        let account = self
            .client
            .get_account_state(self.config.bridge_address.clone())
            .await?
            .0
            .ok_or(BlockProviderError::AccountNotFound)?;

        let context = ExecutionContextBuilder::new(&account)
            .with_config(self.blockchain_config.clone())
            .build()?;

        let result = context.run_getter("get_state_short", &[])?;
        if !result.success {
            return Err(BlockProviderError::VmExecutionFailed(result.exit_code).into());
        }

        let current_epoch_since: u32 = result.stack[0].try_as_int()?.try_into()?;
        Ok(current_epoch_since)
    }
}

#[derive(Debug)]
pub struct KeyBlockData {
    pub prev_seqno: u32,
    pub v_set: everscale_types::models::ValidatorSet,
    pub signatures: Vec<everscale_types::models::BlockSignature>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockProviderConfig {
    pub bridge_address: StdAddr,
    #[serde(with = "serde_helpers::humantime")]
    pub polling_timeout: Duration,
    #[serde(with = "serde_helpers::humantime")]
    pub error_timeout: Duration,
}

#[derive(thiserror::Error, Debug)]
pub enum BlockProviderError {
    #[error("account state not found")]
    AccountNotFound,
    #[error("vm execution failed: {0}")]
    VmExecutionFailed(i32),
}
