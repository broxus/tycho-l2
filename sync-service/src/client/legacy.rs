use anyhow::{Context, Result};
use async_trait::async_trait;
use proof_api_util::block::{
    BlockchainBlock, BlockchainBlockExtra, BlockchainBlockMcExtra, BlockchainModels, LegacyModels,
    make_key_block_proof,
};
use proof_api_util::serde_helpers::gql_shard_prefix;
use reqwest::{IntoUrl, Url};
use serde::{Deserialize, Serialize};
use tycho_types::cell::Lazy;
use tycho_types::models::{
    BlockId, BlockSignature, BlockchainConfig, ShardIdent, Signature, SignatureContext, StdAddr,
    Transaction,
};
use tycho_types::prelude::*;
use tycho_util::serde_helpers;

use crate::client::{DataToSign, KeyBlockData, NetworkClient};
use crate::util::account::AccountStateResponse;

pub struct LegacyClient {
    name: String,
    client: reqwest::Client,
    base_url: Url,
}

impl LegacyClient {
    pub fn new(name: impl Into<String>, base_url: impl IntoUrl) -> Result<Self> {
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
            name: name.into(),
            client,
            base_url: base_url.into_url()?,
        })
    }

    fn post<R>(
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

#[async_trait]
impl NetworkClient for LegacyClient {
    fn name(&self) -> &str {
        &self.name
    }

    async fn get_signature_context(&self) -> Result<SignatureContext> {
        Err(anyhow::anyhow!("unimplemented"))
    }

    async fn get_latest_key_block_seqno(&self) -> Result<u32> {
        #[allow(unused)]
        #[derive(Deserialize)]
        struct KeyBlocks {
            key_blocks: Edges<KeyBlockInfo>,
        }

        #[derive(Deserialize)]
        struct KeyBlockInfo {
            seq_no: u32,
        }

        let Bc::<KeyBlocks> { blockchain } = self
            .post("{blockchain{key_blocks(last:1){edges{node{seq_no}}}}}".to_owned())
            .await?;

        let Some(item) = blockchain.key_blocks.edges.first() else {
            anyhow::bail!("no keyblocks found");
        };
        Ok(item.node.seq_no)
    }

    async fn get_blockchain_config(&self) -> Result<BlockchainConfig> {
        #[allow(unused)]
        #[derive(Deserialize)]
        struct KeyBlocks {
            key_blocks: Edges<KeyBlockInfo>,
        }

        #[derive(Deserialize)]
        struct KeyBlockInfo {
            #[serde(with = "Boc")]
            boc: Cell,
        }

        let Bc::<KeyBlocks> { blockchain } = self
            .post("{blockchain{key_blocks(last:1){edges{node{boc}}}}}".to_owned())
            .await?;

        let Some(item) = blockchain.key_blocks.edges.first() else {
            anyhow::bail!("no keyblocks found");
        };
        let extra = item
            .node
            .boc
            .parse::<<LegacyModels as BlockchainModels>::Block>()?
            .load_extra()
            .context("failed to parse block extra")?
            .load_custom()
            .context("failed to parse mc extra")?
            .context("mc extra expected for masterchain block")?;
        let Some(config) = extra.config() else {
            anyhow::bail!("keyblock must contain config");
        };
        Ok(config.clone())
    }

    async fn get_library_cell(&self, _lib_hash: &HashBytes) -> Result<Option<Cell>> {
        Err(anyhow::anyhow!("unimplemented"))
    }

    async fn get_key_block(&self, seqno: u32) -> Result<KeyBlockData> {
        #[allow(unused)]
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

        let res = self
            .post::<Bc<BlkBySeqno>>(format!(
                "{{blockchain{{block_by_seq_no(\
                workchain:-1,shard:\"{:016x}\",seq_no:{seqno}){{\
                workchain_id shard seq_no file_hash boc \
                signatures{{signatures{{node_id r s}}}}\
                }}}}}}",
                ShardIdent::MASTERCHAIN.prefix(),
            ))
            .await?
            .blockchain
            .block_by_seq_no;
        let Some(info) = res else {
            anyhow::bail!("not found");
        };

        anyhow::ensure!(info.workchain_id == -1, "block workchain id mismatch");
        anyhow::ensure!(
            info.shard == ShardIdent::MASTERCHAIN.prefix(),
            "block shard mismatch"
        );
        anyhow::ensure!(info.seq_no == seqno, "block seqno mismatch");

        let block_id = BlockId {
            shard: ShardIdent::MASTERCHAIN,
            seqno,
            root_hash: *info.boc.repr_hash(),
            file_hash: info.file_hash,
        };

        let block = info
            .boc
            .parse::<<LegacyModels as BlockchainModels>::Block>()?;
        let prev_key_block_seqno = block.load_info()?.prev_key_block_seqno;

        let custom = block
            .load_extra()?
            .load_custom()?
            .context("expected McBlockCustom")?;
        let config = custom.config().context("expected config")?;

        let signatures = info
            .signatures
            .signatures
            .context("key block signatures not found")?
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
            .collect();

        Ok(KeyBlockData {
            block_id,
            root: info.boc,
            prev_key_block_seqno,
            current_vset: config.get_current_validator_set()?,
            prev_vset: config.get_previous_validator_set()?,
            data_to_sign: DataToSign::Ordinary,
            signatures,
        })
    }

    async fn get_account_state(&self, _: &StdAddr, _: Option<u64>) -> Result<AccountStateResponse> {
        Err(anyhow::anyhow!("unimplemented"))
    }

    async fn get_transactions(
        &self,
        _: &StdAddr,
        _: u64,
        _: &HashBytes,
        _: u8,
    ) -> Result<Vec<Lazy<Transaction>>> {
        Err(anyhow::anyhow!("unimplemented"))
    }

    async fn send_message(&self, _: Cell) -> Result<()> {
        Err(anyhow::anyhow!("unimplemented"))
    }

    fn make_key_block_proof_to_sync(&self, data: &KeyBlockData) -> Result<Cell> {
        make_key_block_proof::<LegacyModels>(
            data.root.clone(),
            data.prev_vset
                .as_ref()
                .map(|prev_vset| data.current_vset.utime_since != prev_vset.utime_until)
                .unwrap_or_default(),
        )
        .context("failed to build key block proof")
    }
}

#[derive(Deserialize)]
struct Edges<T> {
    edges: Vec<EdgeNode<T>>,
}

#[derive(Deserialize)]
struct EdgeNode<T> {
    node: T,
}

#[derive(Deserialize)]
struct Bc<T> {
    blockchain: T,
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
mod test {
    use super::*;

    #[tokio::test]
    async fn legacy_client_works() -> Result<()> {
        let client = LegacyClient::new("c", "https://gql.venom.foundation/graphql")?;

        let config = client.get_blockchain_config().await?;
        assert_eq!(config.address, config.get_raw(0)?.unwrap().load_u256()?);

        let seqno = client.get_latest_key_block_seqno().await?;
        let key_block = client.get_key_block(seqno).await?;
        println!("{seqno}: {}", key_block.block_id);

        let proof = client.make_key_block_proof_to_sync(&key_block)?;
        println!("{}", Boc::encode_base64(proof));

        Ok(())
    }
}
