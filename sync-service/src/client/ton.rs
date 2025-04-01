use anyhow::{Context, Result};
use async_trait::async_trait;
use everscale_types::cell::Lazy;
use everscale_types::error::Error;
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{
    BlockIdShort, BlockchainConfig, CurrencyCollection, OptionalAccount, ShardAccounts,
    ShardHashes, ShardIdent, StdAddr, Transaction,
};
use everscale_types::prelude::*;
use proof_api_util::block::{
    make_key_block_proof, BlockchainBlock, BlockchainBlockExtra, BlockchainBlockMcExtra,
    BlockchainModels, TonModels,
};
use ton_lite_client::{proto, LiteClient};

use crate::client::{KeyBlockData, NetworkClient};
use crate::util::account::{AccountStateResponse, GenTimings, LastTransactionId};

pub struct TonClient {
    name: String,
    rpc: LiteClient,
}

impl TonClient {
    pub fn new(name: impl Into<String>, rpc: LiteClient) -> Self {
        Self {
            name: name.into(),
            rpc,
        }
    }
}

#[async_trait]
impl NetworkClient for TonClient {
    fn name(&self) -> &str {
        &self.name
    }

    async fn get_signature_id(&self) -> Result<Option<i32>> {
        Ok(None)
    }

    async fn get_latest_key_block_seqno(&self) -> Result<u32> {
        let mc_block_id = self.rpc.get_last_mc_block_id().await?;

        let mc_block = self
            .rpc
            .get_block(&mc_block_id)
            .await?
            .parse::<<TonModels as BlockchainModels>::Block>()?;

        let info = mc_block.load_info()?;
        Ok(if info.is_key_block {
            mc_block_id.seqno
        } else {
            info.prev_key_block_seqno
        })
    }

    async fn get_blockchain_config(&self) -> Result<BlockchainConfig> {
        let mc_block_id = self.rpc.get_last_mc_block_id().await?;
        let config = self.rpc.get_config(&mc_block_id).await?;

        let state_proof = Boc::decode(&config.config_proof)?
            .parse_exotic::<MerkleProof>()?
            .cell;

        let mut cs: CellSlice<'_> = state_proof.as_slice()?;
        cs.only_last(1, 1)?;
        let extra = <Option<Cell>>::load_from(&mut cs)
            .context("failed to read McStateExtra")?
            .context("expected McStateExtra")?
            .parse::<TonMcStateExtraShort>()?;

        Ok(extra.config)
    }

    async fn get_key_block(&self, seqno: u32) -> Result<KeyBlockData> {
        let rpc = &self.rpc;

        let key_block_id = rpc
            .lookup_block(BlockIdShort {
                shard: ShardIdent::MASTERCHAIN,
                seqno,
            })
            .await?;

        let root = rpc.get_block(&key_block_id).await?;
        let block = root.parse::<<TonModels as BlockchainModels>::Block>()?;
        let prev_key_block_seqno = block.load_info()?.prev_key_block_seqno;

        let custom = block
            .load_extra()?
            .load_custom()?
            .context("expected McBlockCustom")?;
        let config = custom.config().context("expected config")?;

        // TODO: Handle zerostate.
        let prev_key_block_id = rpc
            .lookup_block(BlockIdShort {
                shard: ShardIdent::MASTERCHAIN,
                seqno: prev_key_block_seqno,
            })
            .await?;

        let proof = 'proof: {
            let partial = rpc
                .get_block_proof(&prev_key_block_id, Some(&key_block_id), false)
                .await?;

            for step in partial.steps {
                if let proto::BlockLink::BlockLinkForward(proof) = step {
                    anyhow::ensure!(proof.to == key_block_id, "proof block id mismatch");
                    break 'proof proof;
                }
            }

            anyhow::bail!("key block proof not found");
        };

        Ok(KeyBlockData {
            block_id: key_block_id,
            root,
            prev_key_block_seqno,
            signatures: proof.signatures.signatures,
            current_vset: config.get_current_validator_set()?,
            prev_vset: config.get_previous_validator_set()?,
        })
    }

    async fn get_account_state(
        &self,
        account: &StdAddr,
        last_transaction_lt: Option<u64>,
    ) -> Result<AccountStateResponse> {
        let mc_block_id = self.rpc.get_last_mc_block_id().await?;
        let account_state = self.rpc.get_account(&mc_block_id, account).await?;

        let proofs = parse_proofs(account_state.proof)?;
        if account_state.state.is_empty() {
            return Ok(AccountStateResponse::NotExists {
                timings: proofs.timings,
            });
        }

        let last_transaction_id = proofs
            .get_last_transaction_id(&account.address)
            .context("failed to get last transaction id")?;
        if let Some(lt) = last_transaction_lt {
            if last_transaction_id.lt == lt {
                return Ok(AccountStateResponse::Unchanged {
                    timings: proofs.timings,
                });
            }
        }

        let cell = Boc::decode(&account_state.state)?;
        let OptionalAccount(Some(account)) = cell.parse()? else {
            return Ok(AccountStateResponse::NotExists {
                timings: proofs.timings,
            });
        };

        Ok(AccountStateResponse::Exists {
            account: Box::new(account),
            timings: proofs.timings,
            last_transaction_id,
        })
    }

    async fn get_transactions(
        &self,
        account: &StdAddr,
        lt: u64,
        hash: &HashBytes,
        count: u8,
    ) -> Result<Vec<Lazy<Transaction>>> {
        use everscale_types::boc::de::{BocHeader, Options};

        let res = self
            .rpc
            .get_transactions(account, lt, hash, count as u32)
            .await?;

        let header = BocHeader::decode(&res.transactions, &Options {
            min_roots: None,
            max_roots: None,
        })
        .context("failed to deserialize transactions")?;

        let roots = header.roots().to_vec();
        let cells = header.finalize(Cell::empty_context())?;

        let mut result = Vec::with_capacity(roots.len());
        for root in roots {
            let tx = cells.get(root).context("tx root not found")?;
            result.push(Lazy::from_raw(tx)?);
        }

        Ok(result)
    }

    async fn send_message(&self, message: Cell) -> Result<()> {
        let status = self.rpc.send_message(Boc::encode(message)).await?;
        anyhow::ensure!(status == 1, "message not sent");
        Ok(())
    }

    fn make_key_block_proof_to_sync(&self, data: &KeyBlockData) -> Result<Cell> {
        make_key_block_proof::<TonModels>(
            data.root.clone(),
            data.prev_vset
                .as_ref()
                .map(|prev_vset| data.current_vset.utime_since != prev_vset.utime_until)
                .unwrap_or_default(),
        )
        .context("failed to build key block proof")
    }
}

fn parse_proofs(proofs: Vec<u8>) -> Result<ParsedProofs> {
    use everscale_types::boc::de::{BocHeader, Options};

    let header = BocHeader::decode(&proofs, &Options {
        max_roots: Some(2),
        min_roots: Some(2),
    })?;

    let block_proof_id = *header.roots().first().context("block proof not found")?;
    let state_proof_id = *header.roots().get(1).context("state proof not found")?;
    let cells = header.finalize(Cell::empty_context())?;

    let block = cells
        .get(block_proof_id)
        .context("block proof not found")?
        .parse_exotic::<MerkleProof>()?
        .cell
        .parse::<<TonModels as BlockchainModels>::Block>()?;

    let info = block.load_info()?;
    let timings = GenTimings {
        gen_lt: info.end_lt,
        gen_utime: info.gen_utime,
    };

    let state_root = cells
        .get(state_proof_id)
        .context("state proof not found")?
        .parse_exotic::<MerkleProof>()?
        .cell;

    Ok(ParsedProofs {
        timings,
        state_root,
    })
}

struct ParsedProofs {
    timings: GenTimings,
    state_root: Cell,
}

impl ParsedProofs {
    fn get_last_transaction_id(&self, account: &HashBytes) -> Result<LastTransactionId> {
        type ShardAccountsShort = Dict<HashBytes, TonShardAccountShort>;

        let proof = self
            .state_root
            .parse::<TonShardStateShort>()
            .context("invalid state proof")?;

        let accounts = proof
            .accounts
            .parse::<ShardAccounts>()
            .context("failed to parse shard accounts")?;
        let accounts = ShardAccountsShort::from_raw(accounts.dict().root().clone());

        let Some(state) = accounts.get(account).context("failed to get tx id")? else {
            anyhow::bail!("account state not found");
        };

        Ok(LastTransactionId {
            hash: state.last_trans_hash,
            lt: state.last_trans_lt,
        })
    }
}

#[derive(Load)]
#[tlb(tag = "#9023afe2")]
struct TonShardStateShort {
    _out_msg_queue_info: Cell,
    accounts: Cell,
}

struct TonShardAccountShort {
    last_trans_hash: HashBytes,
    last_trans_lt: u64,
}

impl<'a> Load<'a> for TonShardAccountShort {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        // Skip `split_depth`
        slice.skip_first(5, 0)?;
        // Skip balance.
        _ = CurrencyCollection::load_from(slice)?;
        // Skip account.
        Cell::load_from(slice)?;

        Ok(Self {
            last_trans_hash: slice.load_u256()?,
            last_trans_lt: slice.load_u64()?,
        })
    }
}

#[derive(Load)]
#[tlb(tag = "#cc26")]
struct TonMcStateExtraShort {
    _shard_hashes: ShardHashes,
    config: BlockchainConfig,
}
