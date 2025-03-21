use everscale_types::cell::{CellSlice, Lazy, Load};
use everscale_types::error::Error;
use everscale_types::models::ShardIdent;

macro_rules! ok {
    ($e:expr $(,)?) => {
        match $e {
            core::result::Result::Ok(val) => val,
            core::result::Result::Err(err) => return core::result::Result::Err(err),
        }
    };
}

/// Shard block.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Block {
    /// Global network id.
    pub global_id: i32,
    /// Block info.
    pub info: Lazy<BlockInfo>,
}

impl Block {
    const TAG_V1: u32 = 0x11ef55aa;
    const TAG_V2: u32 = 0x11ef55bb;

    /// Tries to load block info.
    pub fn load_info(&self) -> Result<BlockInfo, Error> {
        self.info.load()
    }
}

impl<'a> Load<'a> for Block {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        match ok!(slice.load_u32()) {
            Self::TAG_V1 | Self::TAG_V2 => (),
            _ => return Err(Error::InvalidTag),
        };

        let global_id = ok!(slice.load_u32()) as i32;
        let info = ok!(Lazy::load_from(slice));

        Ok(Self { global_id, info })
    }
}

/// Block info.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BlockInfo {
    /// Block model version.
    pub version: u32,
    /// Whether this block was produced after the shards were merged.
    pub after_merge: bool,
    /// Whether this block was produced before the shards split.
    pub before_split: bool,
    /// Whether this block was produced after the shards split.
    pub after_split: bool,
    /// Hint that the shard with this block should split.
    pub want_split: bool,
    /// Hint that the shard with this block should merge.
    pub want_merge: bool,
    /// Whether this block is a key block.
    pub key_block: bool,

    /// Block flags (currently only bit 1 is used, for [`gen_software`])
    ///
    /// [`gen_software`]: Self::gen_software
    pub flags: u8,
    /// Block sequence number.
    pub seqno: u32,
    /// Block vertical sequence number.
    pub vert_seqno: u32,

    /// Shard id where this block was produced.
    pub shard: ShardIdent,
    /// Unix timestamp when the block was created.
    pub gen_utime: u32,
    /// Logical time range start.
    pub start_lt: u64,
    /// Logical time range end.
    pub end_lt: u64,
    /// Last 4 bytes of the hash of the validator list.
    pub gen_validator_list_hash_short: u32,
    /// Seqno of the catchain session where this block was produced.
    pub gen_catchain_seqno: u32,
    /// Minimal referenced seqno of the masterchain block.
    pub min_ref_mc_seqno: u32,
    /// Previous key block seqno.
    pub prev_key_block_seqno: u32,
}

impl BlockInfo {
    const TAG_V1: u32 = 0x9bc7a987;
}

impl<'a> Load<'a> for BlockInfo {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        match slice.load_u32() {
            Ok(Self::TAG_V1) => (),
            Ok(_) => return Err(Error::InvalidTag),
            Err(e) => return Err(e),
        };

        let version = ok!(slice.load_u32());
        let [packed_flags, flags] = ok!(slice.load_u16()).to_be_bytes();
        let seqno = ok!(slice.load_u32());
        if seqno == 0 {
            return Err(Error::InvalidData);
        }
        let vert_seqno = ok!(slice.load_u32());
        let shard = ok!(ShardIdent::load_from(slice));
        let gen_utime = ok!(slice.load_u32());
        let start_lt = ok!(slice.load_u64());
        let end_lt = ok!(slice.load_u64());
        let gen_validator_list_hash_short = ok!(slice.load_u32());
        let gen_catchain_seqno = ok!(slice.load_u32());
        let min_ref_mc_seqno = ok!(slice.load_u32());
        let prev_key_block_seqno = ok!(slice.load_u32());

        Ok(Self {
            version,
            after_merge: packed_flags & 0b01000000 != 0,
            before_split: packed_flags & 0b00100000 != 0,
            after_split: packed_flags & 0b00010000 != 0,
            want_split: packed_flags & 0b00001000 != 0,
            want_merge: packed_flags & 0b00000100 != 0,
            key_block: packed_flags & 0b00000010 != 0,
            flags,
            seqno,
            vert_seqno,
            shard,
            gen_utime,
            start_lt,
            end_lt,
            gen_validator_list_hash_short,
            gen_catchain_seqno,
            min_ref_mc_seqno,
            prev_key_block_seqno,
        })
    }
}
