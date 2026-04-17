use anyhow::{Context, Result};
use proof_api_util::block::{
    self, BlockchainBlock, BlockchainBlockExtra, BlockchainBlockMcExtra, BlockchainModels,
    DataToSign, LegacyModels,
};
use proof_api_util::serde_helpers::{gql_shard_prefix, gql_u64};
use reqwest::{IntoUrl, Url};
use serde::{Deserialize, Serialize};
use tycho_types::models::{BlockId, BlockIdShort, BlockSignature, ShardIdent, Signature, StdAddr};
use tycho_types::prelude::*;
use tycho_util::serde_helpers;

pub struct LegacyClient {
    client: reqwest::Client,
    base_url: Url,
}

impl LegacyClient {
    pub fn new<U: IntoUrl>(base_url: U) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        let client = reqwest::ClientBuilder::new()
            .default_headers(headers)
            .build()
            .context("failed to build http client")?;

        Ok(Self {
            client,
            base_url: base_url.into_url()?,
        })
    }

    // TODO: Add keyblocks cache
    pub async fn build_proof(
        &self,
        account: &StdAddr,
        lt: u64,
        tx_hash: &HashBytes,
    ) -> Result<Cell> {
        let Some(tx_info) = self.get_tx_info(tx_hash).await? else {
            anyhow::bail!("transaction not found");
        };
        anyhow::ensure!(tx_info.lt == lt, "transaction lt mismatch");
        anyhow::ensure!(
            &tx_info.account_addr == account,
            "transaction account mismatch"
        );
        tracing::debug!(block_id = %tx_info.block_id, "found transaction block id");

        let is_masterchain = account.is_masterchain();

        let Some(loaded) = self
            .get_block(BlockQueryBy::Hash(&tx_info.block_id))
            .await?
        else {
            anyhow::bail!("transaction block not found");
        };

        let prev_key_block_seqno = loaded
            .root
            .parse::<<LegacyModels as BlockchainModels>::Block>()?
            .load_info()?
            .prev_key_block_seqno;

        let tx_proof =
            block::make_tx_proof::<LegacyModels>(loaded.root, &account.address, lt, is_masterchain)
                .context("failed to build tx proof")?
                .context("tx not found in block")?;

        let vset = {
            let Some(key_block) = self
                .get_block(BlockQueryBy::Seqno(BlockIdShort {
                    shard: ShardIdent::MASTERCHAIN,
                    seqno: prev_key_block_seqno,
                }))
                .await?
            else {
                anyhow::bail!("prev key block not found");
            };

            let Some(custom) = key_block
                .root
                .parse::<<LegacyModels as BlockchainModels>::Block>()?
                .load_extra()?
                .load_custom()?
            else {
                anyhow::bail!("got key block without McBlockExtra");
            };

            let Some(config) = custom.config() else {
                anyhow::bail!("got key block without config");
            };

            config.get_current_validator_set()?
        };

        let mc_proof;
        let file_hash;
        let vset_utime_since = vset.utime_since;
        let signatures;
        let mut shard_proofs = Vec::new();
        if account.is_masterchain() {
            // No shard blocks are required in addition to masterchain proof.
            file_hash = loaded.block_id.file_hash;
            mc_proof = tx_proof;

            let Some(loaded_signatures) = loaded.signatures else {
                anyhow::bail!("masterchain block must contain signatures");
            };

            signatures = block::prepare_signatures(
                &DataToSign::Ordinary,
                loaded_signatures.into_iter().map(Ok),
                &vset,
            )
            .context("failed to prepare block signatures")?;
        } else {
            let Some(loaded_mc_block) = self
                .get_block(BlockQueryBy::Seqno(BlockIdShort {
                    shard: ShardIdent::MASTERCHAIN,
                    seqno: tx_info.master_seq_no,
                }))
                .await?
            else {
                anyhow::bail!("transaction block not found");
            };
            file_hash = loaded_mc_block.block_id.file_hash;

            let Some(loaded_signatures) = loaded_mc_block.signatures else {
                anyhow::bail!("masterchain block must contain signatures");
            };

            signatures = block::prepare_signatures(
                &DataToSign::Ordinary,
                loaded_signatures.into_iter().map(Ok),
                &vset,
            )
            .context("failed to prepare block signatures")?;

            let mc =
                block::make_mc_proof::<LegacyModels>(loaded_mc_block.root, loaded.block_id.shard)
                    .context("failed to build mc block proof")?;
            mc_proof = mc.root;

            for seqno in (loaded.block_id.seqno + 1..=mc.latest_shard_seqno).rev() {
                let Some(loaded_sc_block) = self
                    .get_block(BlockQueryBy::Seqno(BlockIdShort {
                        shard: loaded.block_id.shard,
                        seqno,
                    }))
                    .await?
                else {
                    anyhow::bail!("intermediate shard block not found");
                };

                let proof =
                    block::make_pivot_block_proof::<LegacyModels>(false, loaded_sc_block.root)
                        .context("failed to build pivot block proof")?;
                shard_proofs.push(proof);
            }

            shard_proofs.push(tx_proof);
        };

        let proof_chain = block::make_proof_chain(
            &file_hash,
            mc_proof,
            &shard_proofs,
            vset_utime_since,
            signatures,
        )?;
        Ok(proof_chain)
    }

    async fn get_tx_info(&self, hash: &HashBytes) -> Result<Option<TxInfoBrief>> {
        #[allow(unused)]
        #[derive(Deserialize)]
        struct Tx {
            transaction: Option<TxInfoBrief>,
        }

        let BlockchainResponse::<Tx> { blockchain } = self
            .post(format!(
                "{{blockchain{{transaction(hash:\"{hash}\"){{\
                master_seq_no block_id lt account_addr\
                }}}}}}",
            ))
            .await?;
        Ok(blockchain.transaction)
    }

    async fn get_block(&self, by: BlockQueryBy<'_>) -> Result<Option<LoadedBlockFull>> {
        #[derive(Deserialize)]
        struct BlkByHash {
            block: Option<BlockInfoBrief>,
        }

        #[derive(Deserialize)]
        struct BlkBySeqno {
            block_by_seq_no: Option<BlockInfoBrief>,
        }

        #[derive(Debug, Deserialize)]
        struct BlockInfoBrief {
            workchain_id: i32,
            #[serde(with = "gql_shard_prefix")]
            shard: u64,
            seq_no: u32,
            file_hash: HashBytes,
            #[serde(with = "Boc")]
            boc: Cell,
            signatures: BlockSignatures,
        }

        #[derive(Debug, Deserialize)]
        struct BlockSignatures {
            signatures: Option<Vec<GqlSignaturesItem>>,
        }

        #[derive(Debug, Deserialize)]
        struct GqlSignaturesItem {
            node_id: HashBytes,
            r: HashBytes,
            s: HashBytes,
        }

        let res = match by {
            BlockQueryBy::Hash(hash) => {
                self.post::<BlockchainResponse<BlkByHash>>(format!(
                    "{{blockchain{{block(hash:\"{hash}\"){{\
                workchain_id shard seq_no file_hash boc \
                signatures{{signatures{{node_id r s}}}}\
                }}}}}}"
                ))
                .await?
                .blockchain
                .block
            }
            BlockQueryBy::Seqno(id) => {
                self.post::<BlockchainResponse<BlkBySeqno>>(format!(
                    "{{blockchain{{block_by_seq_no(\
                workchain:{},shard:\"{:016x}\",seq_no:{}){{\
                workchain_id shard seq_no file_hash boc \
                signatures{{signatures{{node_id r s}}}}\
                }}}}}}",
                    id.shard.workchain(),
                    id.shard.prefix(),
                    id.seqno,
                ))
                .await?
                .blockchain
                .block_by_seq_no
            }
        };
        let Some(info) = res else {
            return Ok(None);
        };

        let block_id = match by {
            BlockQueryBy::Hash(hash) => {
                anyhow::ensure!(info.boc.repr_hash() == hash, "block root hash mismatch");
                let Some(shard) = ShardIdent::new(info.workchain_id, info.shard) else {
                    anyhow::bail!("invalid shard");
                };
                BlockId {
                    shard,
                    seqno: info.seq_no,
                    root_hash: *hash,
                    file_hash: info.file_hash,
                }
            }
            BlockQueryBy::Seqno(id) => {
                anyhow::ensure!(
                    info.workchain_id == id.shard.workchain(),
                    "block workchain id mismatch"
                );
                anyhow::ensure!(info.shard == id.shard.prefix(), "block shard mismatch");
                anyhow::ensure!(info.seq_no == id.seqno, "block seqno mismatch");

                BlockId {
                    shard: id.shard,
                    seqno: info.seq_no,
                    root_hash: *info.boc.repr_hash(),
                    file_hash: info.file_hash,
                }
            }
        };

        Ok(Some(LoadedBlockFull {
            block_id,
            root: info.boc,
            signatures: info.signatures.signatures.map(|items| {
                items
                    .into_iter()
                    .map(|item| BlockSignature {
                        node_id_short: item.node_id,
                        signature: {
                            let mut s = Signature([0; 64]);
                            s.0[0..32].copy_from_slice(item.r.as_slice());
                            s.0[32..64].copy_from_slice(item.s.as_slice());
                            s
                        },
                    })
                    .collect()
            }),
        }))
    }

    pub fn post<R>(
        &self,
        query: impl std::fmt::Display,
    ) -> impl Future<Output = Result<R>> + Send + 'static
    where
        for<'de> R: Deserialize<'de> + Send + 'static,
    {
        let send = self
            .client
            .post(self.base_url.clone())
            .json(&GqlRequest { query })
            .send();

        async move {
            let response = send.await?;

            let res = response.text().await?;
            tracing::trace!(res);

            let GqlResponse { data } = serde_json::from_str(&res).context("invalid response")?;
            Ok(data)
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum BlockQueryBy<'a> {
    Hash(&'a HashBytes),
    Seqno(BlockIdShort),
}

#[derive(Deserialize)]
struct BlockchainResponse<T> {
    blockchain: T,
}

#[derive(Debug, Deserialize)]
struct TxInfoBrief {
    master_seq_no: u32,
    block_id: HashBytes,
    #[serde(with = "gql_u64")]
    lt: u64,
    account_addr: StdAddr,
}

struct LoadedBlockFull {
    block_id: BlockId,
    root: Cell,
    signatures: Option<Vec<BlockSignature>>,
}

#[derive(Serialize)]
struct GqlRequest<T: std::fmt::Display> {
    #[serde(with = "serde_helpers::string")]
    query: T,
}

#[derive(Deserialize)]
struct GqlResponse<T> {
    data: T,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn client_works() -> Result<()> {
        // NOTE: Why is it still alive? :(
        let client = LegacyClient::new("https://gql.venom.foundation/graphql")?;
        let res = client
            .build_proof(
                &"0:0c46cd845d480a472d5c6085825cb7859e7d5fc0678f776ff8ad059c46c167f8"
                    .parse::<StdAddr>()?,
                160246494000001,
                &"2a28aa225e8b089397175b4f51bb53f1e93dcc3f532521e8526031d86c1d53e1"
                    .parse::<HashBytes>()?,
            )
            .await?;

        println!("{}", Boc::encode_base64(res));
        Ok(())
    }
}
