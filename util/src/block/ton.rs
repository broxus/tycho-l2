use tycho_types::error::Error;
use tycho_types::models::{
    BlockIdShort, BlockSignature, BlockchainConfig, GlobalVersion, ShardHashes, ShardIdent,
    ValidatorBaseInfo,
};
use tycho_types::prelude::*;

use crate::block::{
    AccountBlocksShort, BlockchainBlock, BlockchainBlockExtra, BlockchainBlockInfo,
    BlockchainBlockMcExtra, BlockchainBlockSignatures, BlockchainModels, find_shard_descr,
};

pub struct TonModels;

impl BlockchainModels for TonModels {
    type Block = TonBlock;
    type BlockSignatures = TonBlockSignatures;
}

#[derive(Load)]
#[tlb(tag = "#11ef55aa")]
pub struct TonBlock {
    pub global_id: i32,
    pub info: Cell,
    pub value_flow: Cell,
    pub state_update: Cell,
    pub extra: Cell,
}

impl BlockchainBlock for TonBlock {
    type Info = TonBlockInfo;
    type Extra = TonBlockExtra;

    fn load_info(&self) -> Result<Self::Info, Error> {
        self.info.parse::<Self::Info>()
    }

    fn load_info_raw(&self) -> Result<Cell, Error> {
        Ok(self.info.clone())
    }

    fn load_extra(&self) -> Result<Self::Extra, Error> {
        self.extra.parse::<Self::Extra>()
    }
}

pub struct TonBlockInfo {
    pub is_key_block: bool,
    pub shard: ShardIdent,
    pub gen_utime: u32,
    pub start_lt: u64,
    pub end_lt: u64,
    pub prev_key_block_seqno: u32,
    pub master_ref: Option<Cell>,
    pub prev_ref: Cell,
    pub prev_vert_ref: Option<Cell>,
}

impl TonBlockInfo {
    const TAG: u32 = 0x9bc7a987;
    const FLAG_WITH_GEN_SOFTWARE: u8 = 0x1;
}

impl<'a> Load<'a> for TonBlockInfo {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        if slice.load_u32()? != Self::TAG {
            return Err(Error::InvalidTag);
        };

        let _version = slice.load_u32()?;
        let [packed_flags, flags] = slice.load_u16()?.to_be_bytes();
        let seqno = slice.load_u32()?;
        if seqno == 0 {
            return Err(Error::InvalidData);
        }

        let is_key_block = packed_flags & 0b00000010 != 0;

        let vert_seqno = slice.load_u32()?;
        let shard = ShardIdent::load_from(slice)?;
        let gen_utime = slice.load_u32()?;

        let start_lt = slice.load_u64()?;
        let end_lt = slice.load_u64()?;

        let _gen_validator_list_hash_short = slice.load_u32()?;
        let _gen_catchain_seqno = slice.load_u32()?;
        let _min_ref_mc_seqno = slice.load_u32()?;
        let prev_key_block_seqno = slice.load_u32()?;

        if flags & Self::FLAG_WITH_GEN_SOFTWARE != 0 {
            GlobalVersion::load_from(slice)?;
        }

        let master_ref = if packed_flags & 0b10000000 != 0 {
            Some(slice.load_reference_cloned()?)
        } else {
            None
        };

        let prev_ref = slice.load_reference_cloned()?;

        let prev_vert_ref = if packed_flags & 0b00000001 != 0 {
            Some(slice.load_reference_cloned()?)
        } else {
            None
        };

        if vert_seqno < prev_vert_ref.is_some() as u32 {
            return Err(Error::InvalidData);
        }

        Ok(Self {
            is_key_block,
            shard,
            gen_utime,
            start_lt,
            end_lt,
            prev_key_block_seqno,
            master_ref,
            prev_ref,
            prev_vert_ref,
        })
    }
}

impl BlockchainBlockInfo for TonBlockInfo {
    fn is_key_block(&self) -> bool {
        self.is_key_block
    }

    fn end_lt(&self) -> u64 {
        self.end_lt
    }

    fn prev_ref(&self) -> &Cell {
        &self.prev_ref
    }
}

#[derive(Load)]
#[tlb(tag = "#4a33f6fd")]
pub struct TonBlockExtra {
    pub in_msg_description: Cell,
    pub out_msg_description: Cell,
    pub account_blocks: Cell,
    pub rand_seed: HashBytes,
    pub created_by: HashBytes,
    pub custom: Option<Cell>,
}

impl BlockchainBlockExtra for TonBlockExtra {
    type McExtra = TonBlockMcExtra;

    fn load_account_blocks(&self) -> Result<AccountBlocksShort, Error> {
        self.account_blocks.parse::<AccountBlocksShort>()
    }

    fn has_custom(&self) -> bool {
        self.custom.is_some()
    }

    fn load_custom(&self) -> Result<Option<Self::McExtra>, Error> {
        let Some(custom) = self.custom.as_ref() else {
            return Ok(None);
        };
        custom.parse::<Self::McExtra>().map(Some)
    }
}

pub struct TonBlockMcExtra {
    shard_hashes: ShardHashes,
    config: Option<BlockchainConfig>,
}

impl TonBlockMcExtra {
    const TAG: u16 = 0xcca5;
}

impl<'a> Load<'a> for TonBlockMcExtra {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        if slice.load_u16()? != Self::TAG {
            return Err(Error::InvalidTag);
        }

        let with_config = slice.load_bit()?;
        let shard_hashes = ShardHashes::load_from(slice)?;

        let config = if with_config {
            slice.only_last(256, 1)?;
            Some(BlockchainConfig::load_from(slice)?)
        } else {
            None
        };

        Ok(Self {
            shard_hashes,
            config,
        })
    }
}

impl BlockchainBlockMcExtra for TonBlockMcExtra {
    fn load_top_shard_block_ids(&self) -> Result<Vec<BlockIdShort>, Error> {
        let mut shard_ids = Vec::new();
        for entry in self.shard_hashes.latest_blocks() {
            let block_id = entry?;
            shard_ids.push(block_id.as_short_id());
        }

        Ok(shard_ids)
    }

    fn find_shard_seqno(&self, shard_ident: ShardIdent) -> Result<u32, Error> {
        let shard_hashes = self
            .shard_hashes
            .get_workchain_shards(shard_ident.workchain())?
            .ok_or(Error::CellUnderflow)?;

        let mut descr_root = find_shard_descr(shard_hashes.root(), shard_ident.prefix())?;
        let latest_shard_seqno = match descr_root.load_small_uint(4)? {
            0xa | 0xb => descr_root.load_u32()?,
            _ => return Err(Error::InvalidTag),
        };

        Ok(latest_shard_seqno)
    }

    fn visit_all_shard_hashes(&self) -> Result<(), Error> {
        for item in self.shard_hashes.raw_iter() {
            item?;
        }
        Ok(())
    }

    fn config(&self) -> Option<&BlockchainConfig> {
        self.config.as_ref()
    }
}

#[derive(Load)]
#[tlb(tag = "#11")]
pub struct TonBlockSignatures {
    pub validator_info: ValidatorBaseInfo,
    pub signature_count: u32,
    pub total_weight: u64,
    pub signatures: Dict<u16, BlockSignature>,
}

impl BlockchainBlockSignatures for TonBlockSignatures {
    fn validator_info(&self) -> ValidatorBaseInfo {
        self.validator_info
    }

    fn signature_count(&self) -> u32 {
        self.signature_count
    }

    fn total_weight(&self) -> u64 {
        self.total_weight
    }

    fn signatures(&self) -> Dict<u16, BlockSignature> {
        self.signatures.clone()
    }
}
