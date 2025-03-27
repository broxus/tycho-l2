use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use everscale_types::models::{BlockchainConfig, OptionalAccount, StdAddr};
use nekoton_abi::execution_context::ExecutionContextBuilder;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

pub mod ton;
pub mod tycho;

#[async_trait]
pub trait BlockchainClient {
    async fn get_last_key_block(&self) -> anyhow::Result<KeyBlockData>;

    async fn get_key_block(&self, seqno: u32) -> anyhow::Result<KeyBlockData>;

    async fn get_blockchain_config(&self) -> anyhow::Result<BlockchainConfig>;

    async fn get_account_state(&self, account: StdAddr) -> anyhow::Result<OptionalAccount>;
}

pub struct BlockStream<T> {
    client: T,
    config: BlockchainConfig,
    cache: Mutex<BTreeMap<u32, KeyBlockData>>,
    last_known_utime_since: ArcSwapOption<u32>,
    polling_timeout: Duration,
    error_timeout: Duration,
}

impl<T: BlockchainClient> BlockStream<T> {
    pub async fn new(client: T) -> anyhow::Result<Self> {
        let config = client.get_blockchain_config().await?;

        Ok(Self {
            client,
            config,
            cache: Default::default(),
            last_known_utime_since: Default::default(),
            polling_timeout: Duration::from_secs(30),
            error_timeout: Duration::from_secs(1),
        })
    }

    pub async fn next_block(&self) -> Option<KeyBlockData> {
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
                        tokio::time::sleep(self.error_timeout).await;
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
                                tokio::time::sleep(self.error_timeout).await;
                                continue;
                            }
                            _ => return None, // Finish stream (shouldn't happen)
                        }
                    }
                }
                Ok(block_info) if block_info.v_set.utime_since == last_known_utime_since => {
                    tokio::time::sleep(self.polling_timeout).await;
                    continue 'polling;
                }
                Err(e) => {
                    tracing::error!("failed to get last key block: {e}");
                    tokio::time::sleep(self.error_timeout).await;
                    continue 'polling;
                }
                _ => return None, // Finish stream (shouldn't happen)
            }
        }
    }

    async fn get_current_epoch_since(&self) -> anyhow::Result<u32> {
        let addr = StdAddr::from_str(
            "0:457c0ac35986d4e056deee8428abe27294f97c3266dc9062d689a07c8e967164",
        )?; // TODO: move to config

        let account = self
            .client
            .get_account_state(addr)
            .await?
            .0
            .ok_or(BlockStreamError::AccountNotFound)?;

        let context = ExecutionContextBuilder::new(&account)
            .with_config(self.config.clone())
            .build()?;

        let result = context.run_getter("get_state_short", &[])?;
        if !result.success {
            return Err(BlockStreamError::VmExecutionFailed(result.exit_code).into());
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

#[derive(thiserror::Error, Debug)]
pub enum BlockStreamError {
    #[error("account state not found")]
    AccountNotFound,
    #[error("vm execution failed: {0}")]
    VmExecutionFailed(i32),
}
