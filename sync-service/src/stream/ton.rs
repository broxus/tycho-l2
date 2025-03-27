use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use arc_swap::ArcSwapOption;
use everscale_types::boc::Boc;
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{BlockIdShort, BlockchainConfig, ShardIdent};
use everscale_types::prelude::Load;
use parking_lot::Mutex;
use proof_api_util::block::{BlockchainBlock, BlockchainModels, TonModels};
use ton_lite_client::{proto, LiteClient};

use crate::stream::KeyBlockInfo;

pub struct BlockStream {
    client: LiteClient,
    cache: Mutex<BTreeMap<u32, KeyBlockInfo>>,
    last_known_utime_since: ArcSwapOption<u32>,
    polling_timeout: Duration,
    error_timeout: Duration,
}

impl BlockStream {
    pub fn new(client: LiteClient) -> Self {
        Self {
            client,
            cache: Default::default(),
            last_known_utime_since: Default::default(),
            polling_timeout: Duration::from_secs(30),
            error_timeout: Duration::from_secs(1),
        }
    }

    pub async fn next_block(&self) -> Option<KeyBlockInfo> {
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
        drop(cache);

        let last_known_utime_since = match self.last_known_utime_since.load_full() {
            Some(utime_since) => *utime_since,
            None => {
                // TODO: get last known utime_since of validator set from contract
                1742229256
            }
        };

        'polling: loop {
            match get_last_key_block_info(&self.client).await {
                Ok(block_info) if block_info.v_set.utime_since > last_known_utime_since => {
                    let mut prev_key_block_seqno = block_info.prev_seqno;

                    let mut cache = self.cache.lock();
                    cache.insert(block_info.v_set.utime_since, block_info);
                    drop(cache);

                    'traversing: loop {
                        match get_key_block_info(&self.client, prev_key_block_seqno).await {
                            Ok(block_info)
                                if block_info.v_set.utime_since > last_known_utime_since =>
                            {
                                prev_key_block_seqno = block_info.prev_seqno;

                                let mut cache = self.cache.lock();
                                cache.insert(block_info.v_set.utime_since, block_info);
                                drop(cache);

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

async fn get_last_key_block_info(client: &LiteClient) -> Result<KeyBlockInfo> {
    let mc_block_id = client.get_last_mc_block_id().await?;

    let mc_block = client
        .get_block(&mc_block_id)
        .await?
        .parse::<<TonModels as BlockchainModels>::Block>()?;

    let prev_key_block_seqno = mc_block.load_info()?.prev_key_block_seqno;

    get_key_block_info(client, prev_key_block_seqno).await
}

async fn get_key_block_info(client: &LiteClient, key_block_seqno: u32) -> Result<KeyBlockInfo> {
    let key_block_short_id = BlockIdShort {
        shard: ShardIdent::MASTERCHAIN,
        seqno: key_block_seqno,
    };

    let key_block_id = client.lookup_block(key_block_short_id).await?;

    // TODO: Check signatures.
    let key_block_proof = 'proof: {
        let partial = client.get_block_proof(&key_block_id, None).await?;
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

    let prev_key_block_seqno = block.load_info()?.prev_key_block_seqno;

    let custom = block
        .load_extra()?
        .custom
        .ok_or(TonBlockStreamError::KeyBlockNotFull)?;

    let mut slice = custom.as_slice()?;
    slice.only_last(256, 1)?;

    let blockchain_config = BlockchainConfig::load_from(&mut slice)?;
    let v_set = blockchain_config.get_current_validator_set()?;

    let signatures = key_block_proof.signatures.signatures;

    Ok(KeyBlockInfo {
        seqno: key_block_seqno,
        prev_seqno: prev_key_block_seqno,
        v_set,
        signatures,
    })
}

#[derive(thiserror::Error, Debug)]
pub enum TonBlockStreamError {
    #[error("key block not full")]
    KeyBlockNotFull,
    #[error("failed to convert signature")]
    InvalidSignatureLength,
}
