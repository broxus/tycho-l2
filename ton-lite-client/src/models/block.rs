use anyhow::Result;
use everscale_types::cell::{Cell, HashBytes, Load};
use everscale_types::error::Error;
use everscale_types::models::{
    BlockId, BlockInfo, BlockSignature, BlockchainConfig, PrevBlockRef, ValidatorSet,
};

#[derive(Load)]
#[tlb(tag = "#11ef55aa")]
pub struct BlockShort {
    _global_id: i32,
    info: Cell,
    _value_flow: Cell,
    _state_update: Cell,
    extra: Cell,
}

impl BlockShort {
    pub fn load_info(&self) -> Result<BlockInfo, Error> {
        self.info.parse::<BlockInfo>()
    }

    pub fn load_extra(&self) -> Result<BlockExtraShort, Error> {
        self.extra.parse::<BlockExtraShort>()
    }
}

#[derive(Load)]
#[tlb(tag = "#4a33f6fd")]
pub struct BlockExtraShort {
    _in_msg_description: Cell,
    _out_msg_description: Cell,
    _account_blocks: Cell,
    _rand_seed: HashBytes,
    _created_by: HashBytes,
    custom: Option<Cell>,
}

impl BlockExtraShort {
    pub fn load_config(&self) -> Result<BlockchainConfig, Error> {
        let cell = self.custom.as_ref().ok_or(Error::CellUnderflow)?;

        let mut slice = cell.as_slice()?;
        slice.only_last(256, 1)?;

        BlockchainConfig::load_from(&mut slice)
    }
}

pub struct BlockStuff {
    id: BlockId,
    block: BlockShort,
}

impl BlockStuff {
    pub fn new(data: &[u8], id: BlockId) -> Result<Self> {
        let file_hash = everscale_types::boc::Boc::file_hash(data);
        anyhow::ensure!(id.file_hash == file_hash.0, "wrong file_hash");

        let root = everscale_types::boc::Boc::decode(data)?;
        anyhow::ensure!(id.root_hash == root.repr_hash().0, "wrong root hash");

        let block = root.parse::<BlockShort>()?;
        Ok(Self { id, block })
    }

    #[inline(always)]
    pub fn id(&self) -> &BlockId {
        &self.id
    }

    pub fn load_info(&self) -> Result<BlockInfo, Error> {
        self.block.load_info()
    }

    pub fn load_extra(&self) -> Result<BlockExtraShort, Error> {
        self.block.load_extra()
    }

    pub fn construct_prev_id(&self) -> Result<(BlockId, Option<BlockId>)> {
        let header = self.load_info()?;
        match header.load_prev_ref()? {
            PrevBlockRef::Single(prev) => {
                let shard = if header.after_split {
                    let Some(shard) = header.shard.merge() else {
                        anyhow::bail!("failed to merge shard");
                    };
                    shard
                } else {
                    header.shard
                };

                let id = BlockId {
                    shard,
                    seqno: prev.seqno,
                    root_hash: prev.root_hash,
                    file_hash: prev.file_hash,
                };

                Ok((id, None))
            }
            PrevBlockRef::AfterMerge { left, right } => {
                let Some((left_shard, right_shard)) = header.shard.split() else {
                    anyhow::bail!("failed to split shard");
                };

                let id1 = BlockId {
                    shard: left_shard,
                    seqno: left.seqno,
                    root_hash: left.root_hash,
                    file_hash: left.file_hash,
                };

                let id2 = BlockId {
                    shard: right_shard,
                    seqno: right.seqno,
                    root_hash: right.root_hash,
                    file_hash: right.file_hash,
                };

                Ok((id1, Some(id2)))
            }
        }
    }
}

#[derive(Debug)]
pub struct BlockProof {
    pub signatures: Vec<BlockSignature>,
    pub validator_set: ValidatorSet,
}
