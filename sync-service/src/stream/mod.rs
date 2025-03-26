use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use parking_lot::Mutex;

pub mod ton;
pub mod tycho;

#[async_trait]
pub trait BlockchainClient {
    async fn get_last_key_block(&self) -> anyhow::Result<KeyBlockData>;

    async fn get_key_block(&self, seqno: u32) -> anyhow::Result<KeyBlockData>;

    async fn get_last_utime(&self) -> anyhow::Result<u32>;
}

pub struct BlockStream<T> {
    client: T,
    cache: Mutex<BTreeMap<u32, KeyBlockData>>,
    last_known_utime_since: ArcSwapOption<u32>,
    polling_timeout: Duration,
    error_timeout: Duration,
}

impl<T: BlockchainClient> BlockStream<T> {
    pub fn new(client: T) -> Self {
        Self {
            client,
            cache: Default::default(),
            last_known_utime_since: Default::default(),
            polling_timeout: Duration::from_secs(30),
            error_timeout: Duration::from_secs(1),
        }
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
                None => match self.client.get_last_utime().await {
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
}

#[derive(Debug)]
pub struct KeyBlockData {
    pub prev_seqno: u32,
    pub v_set: everscale_types::models::ValidatorSet,
    pub signatures: Vec<everscale_types::models::BlockSignature>,
}
