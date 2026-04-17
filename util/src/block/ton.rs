use anyhow::Result;
use sha2::Digest;
use tl_proto::{IntermediateBytes, TlRead, TlWrite};
use tycho_types::error::Error;
use tycho_types::models::{
    BlockId, BlockIdShort, BlockSignature, BlockchainConfig, GlobalVersion, ShardHashes,
    ShardIdent, ValidatorBaseInfo,
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
    pub gen_catchain_seqno: u32,
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
        let gen_catchain_seqno = slice.load_u32()?;
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
            gen_catchain_seqno,
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

pub fn make_simplex_data_to_sign(
    block_id: &BlockId,
    slot: u32,
    session_id: &[u8; 32],
    candidate: &[u8],
) -> Result<Vec<u8>> {
    let candidate_data = tl_proto::deserialize::<CandidateHashData>(candidate)?;
    anyhow::ensure!(candidate_data.block_id() == block_id, "block id mismatch");

    let candidate_id = CandidateId {
        slot,
        hash: sha2::Sha256::digest(candidate).into(),
    };

    Ok(tl_proto::serialize(SimplexDataToSign {
        session_id,
        data: IntermediateBytes(FinalizeVote { id: &candidate_id }),
    }))
}

#[derive(Debug, TlWrite)]
#[tl(
    boxed,
    id = "consensus.dataToSign",
    scheme_inline = r"
    consensus.dataToSign
        session_id:int256
        data:bytes
        = consensus.DataToSign;"
)]
struct SimplexDataToSign<'a> {
    session_id: &'a [u8; 32],
    data: IntermediateBytes<FinalizeVote<'a>>,
}

#[derive(Debug, TlWrite)]
#[tl(
    boxed,
    id = "consensus.simplex.finalizeVote",
    scheme_inline = r"
    consensus.simplex.finalizeVote
        id:consensus.CandidateId
        = consensus.simplex.UnsignedVote;"
)]
struct FinalizeVote<'a> {
    id: &'a CandidateId,
}

#[derive(Debug, TlRead, TlWrite)]
#[tl(
    boxed,
    scheme_inline = r"
    consensus.candidateHashDataOrdinary
        block:tonNode.blockIdExt
        collated_file_hash:int256
        parent:consensus.CandidateParent
        = consensus.CandidateHashData;
    consensus.candidateHashDataEmpty
        block:tonNode.blockIdExt
        parent:consensus.candidateId
        = consensus.CandidateHashData;"
)]
pub enum CandidateHashData {
    #[tl(id = "consensus.candidateHashDataOrdinary")]
    Ordinary {
        #[tl(with = "tl_block_id_full")]
        block_id: BlockId,
        collated_file_hash: [u8; 32],
        parent: CandidateParent,
    },
    #[tl(id = "consensus.candidateHashDataEmpty")]
    Empty {
        #[tl(with = "tl_block_id_full")]
        block_id: BlockId,
        candidate_id: CandidateId,
    },
}

impl CandidateHashData {
    const TLB_TAG_ORDINARY: u8 = 0x0;
    const TLB_TAG_EMPTY: u8 = 0x1;

    fn block_id(&self) -> &BlockId {
        match self {
            CandidateHashData::Ordinary { block_id, .. }
            | CandidateHashData::Empty { block_id, .. } => block_id,
        }
    }
}

impl Store for CandidateHashData {
    fn store_into(&self, b: &mut CellBuilder, cx: &dyn CellContext) -> Result<(), Error> {
        match self {
            CandidateHashData::Ordinary {
                block_id,
                collated_file_hash,
                parent,
            } => {
                b.store_small_uint(Self::TLB_TAG_ORDINARY, 4)?;
                b.store_u32(block_id.seqno)?;
                b.store_u256(HashBytes::wrap(collated_file_hash))?;
                parent.store_into(b, cx)
            }
            CandidateHashData::Empty {
                block_id,
                candidate_id,
            } => {
                b.store_small_uint(Self::TLB_TAG_EMPTY, 4)?;
                b.store_u32(block_id.seqno)?;
                candidate_id.store_into(b, cx)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, TlRead, TlWrite)]
#[tl(
    boxed,
    scheme_inline = r"
    consensus.candidateParent id:consensus.CandidateId = consensus.CandidateParent;
    consensus.candidateWithoutParents = consensus.CandidateParent;"
)]
pub enum CandidateParent {
    #[tl(id = "consensus.candidateParent")]
    Id(CandidateId),
    #[tl(id = "consensus.candidateWithoutParents")]
    Empty,
}

impl Store for CandidateParent {
    fn store_into(&self, b: &mut CellBuilder, cx: &dyn CellContext) -> Result<(), Error> {
        let candidate_id = match self {
            Self::Id(id) => Some(id),
            Self::Empty => None,
        };
        Option::<&CandidateId>::store_into(&candidate_id, b, cx)
    }
}

#[derive(Debug, Clone, Copy, TlRead, TlWrite)]
#[tl(
    boxed,
    id = "consensus.candidateId",
    scheme_inline = "consensus.candidateId slot:int hash:int256 = consensus.CandidateId;"
)]
pub struct CandidateId {
    pub slot: u32,
    pub hash: [u8; 32],
}

impl Store for CandidateId {
    fn store_into(&self, b: &mut CellBuilder, _: &dyn CellContext) -> Result<(), Error> {
        b.store_u32(self.slot)?;
        b.store_u256(HashBytes::wrap(&self.hash))
    }
}

mod tl_block_id_full {
    use tl_proto::{TlPacket, TlRead, TlResult, TlWrite};
    use tycho_types::models::BlockId;
    use tycho_types::prelude::HashBytes;

    use super::tl_block_id_short;

    pub const SIZE_HINT: usize = tl_block_id_short::SIZE_HINT + 32 + 32;

    pub const fn size_hint(_: &BlockId) -> usize {
        SIZE_HINT
    }

    pub fn write<P: TlPacket>(block_id: &BlockId, packet: &mut P) {
        tl_block_id_short::write(&block_id.as_short_id(), packet);
        block_id.root_hash.0.write_to(packet);
        block_id.file_hash.0.write_to(packet);
    }

    pub fn read(packet: &mut &[u8]) -> TlResult<BlockId> {
        let block_id = tl_block_id_short::read(packet)?;
        let root_hash = HashBytes(<[u8; 32]>::read_from(packet)?);
        let file_hash = HashBytes(<[u8; 32]>::read_from(packet)?);

        Ok(BlockId {
            shard: block_id.shard,
            seqno: block_id.seqno,
            root_hash,
            file_hash,
        })
    }
}

mod tl_block_id_short {
    use tl_proto::{TlPacket, TlRead, TlResult, TlWrite};
    use tycho_types::models::{BlockIdShort, ShardIdent};

    pub const SIZE_HINT: usize = 4 + 8 + 4;

    #[allow(unused)]
    pub const fn size_hint(_: &BlockIdShort) -> usize {
        SIZE_HINT
    }

    pub fn write<P: TlPacket>(block_id: &BlockIdShort, packet: &mut P) {
        block_id.shard.workchain().write_to(packet);
        block_id.shard.prefix().write_to(packet);
        block_id.seqno.write_to(packet);
    }

    pub fn read(packet: &mut &[u8]) -> TlResult<BlockIdShort> {
        let workchain = i32::read_from(packet)?;
        let prefix = u64::read_from(packet)?;
        let seqno = u32::read_from(packet)?;

        let shard = ShardIdent::new(workchain, prefix);

        let shard = match shard {
            None => return Err(tl_proto::TlError::InvalidData),
            Some(shard) => shard,
        };

        Ok(BlockIdShort { shard, seqno })
    }
}
