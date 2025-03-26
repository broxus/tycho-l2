use everscale_types::models::{BlockId, BlockIdShort, StdAddr};
use tl_proto::{IntermediateBytes, TlRead, TlWrite};

#[derive(TlWrite)]
#[tl(boxed, id = "adnl.message.query", scheme = "proto.tl")]
pub struct AdnlMessageQuery<'tl, T> {
    #[tl(size_hint = 32)]
    pub query_id: HashRef<'tl>,
    pub query: IntermediateBytes<LiteQuery<T>>,
}

#[derive(Copy, Clone, TlRead)]
#[tl(boxed, id = "adnl.message.answer", scheme = "proto.tl")]
pub struct AdnlMessageAnswer<'tl> {
    #[tl(size_hint = 32)]
    pub query_id: HashRef<'tl>,
    pub data: &'tl [u8],
}

#[derive(TlWrite)]
#[tl(boxed, id = "liteServer.query", scheme = "proto.tl")]
pub struct LiteQuery<T> {
    pub wrapped_request: IntermediateBytes<WrappedQuery<T>>,
}

#[derive(Debug, TlRead)]
#[tl(boxed, id = "liteServer.masterchainInfo", scheme = "proto.tl")]
pub struct MasterchainInfo {
    #[tl(with = "tl_block_id_full")]
    pub last: BlockId,
    pub state_root_hash: [u8; 32],
    pub init: ZeroStateIdExt,
}

#[derive(Clone, Copy, Debug, TlRead)]
#[tl(boxed, id = "liteServer.version", scheme = "proto.tl")]
pub struct Version {
    pub mode: u32,
    pub version: u32,
    pub capabilities: u64,
    pub now: u32,
}

#[derive(Clone, Copy, Debug, TlRead)]
#[tl(boxed, id = "liteServer.sendMsgStatus", scheme = "proto.tl")]
pub struct SendMsgStatus {
    pub status: u32,
}

#[derive(Debug, Clone, TlRead)]
#[tl(boxed, id = "liteServer.blockData", scheme = "proto.tl")]
pub struct BlockData {
    #[tl(with = "tl_block_id_full")]
    pub id: BlockId,
    pub data: Vec<u8>,
}

#[derive(Debug, TlRead)]
#[tl(boxed, id = "liteServer.blockHeader", scheme = "proto.tl")]
pub struct BlockHeader {
    #[tl(with = "tl_block_id_full")]
    pub id: BlockId,
    #[tl(flags)]
    pub mode: (),
    pub header_proof: Vec<u8>,
}

#[derive(Debug, TlRead)]
#[tl(boxed, id = "liteServer.partialBlockProof", scheme = "proto.tl")]
pub struct PartialBlockProof {
    pub complete: bool,
    #[tl(with = "tl_block_id_full")]
    pub from: BlockId,
    #[tl(with = "tl_block_id_full")]
    pub to: BlockId,
    pub steps: Vec<BlockLink>,
}

#[derive(Debug, TlRead)]
#[tl(boxed, scheme = "proto.tl")]
pub enum BlockLink {
    #[tl(id = "liteServer.blockLinkBack")]
    BlockLinkBack(BlockLinkBack),
    #[tl(id = "liteServer.blockLinkForward")]
    BlockLinkForward(BlockLinkForward),
}

#[derive(Debug, TlRead)]
pub struct BlockLinkBack {
    pub to_key_block: bool,
    #[tl(with = "tl_block_id_full")]
    pub from: BlockId,
    #[tl(with = "tl_block_id_full")]
    pub to: BlockId,
    pub dest_proof: Vec<u8>,
    pub proof: Vec<u8>,
    pub state_proof: Vec<u8>,
}

#[derive(Debug, TlRead)]
pub struct BlockLinkForward {
    pub to_key_block: bool,
    #[tl(with = "tl_block_id_full")]
    pub from: BlockId,
    #[tl(with = "tl_block_id_full")]
    pub to: BlockId,
    pub dest_proof: Vec<u8>,
    pub config_proof: Vec<u8>,
    pub signatures: SignatureSet,
}

#[derive(Debug, TlRead)]
#[tl(boxed, id = "liteServer.signatureSet", scheme = "proto.tl")]
pub struct SignatureSet {
    pub validator_set_hash: u32,
    pub catchain_seqno: u32,
    pub signatures: Vec<Signature>,
}

#[derive(Debug, TlRead)]
// #[tl(boxed, id = "liteServer.signature", scheme = "proto.tl")]
pub struct Signature {
    pub node_id_short: [u8; 32],
    pub signature: Vec<u8>,
}

#[derive(Debug, TlRead)]
#[tl(boxed, id = "liteServer.configInfo", scheme = "proto.tl")]
pub struct ConfigInfo {
    #[tl(flags)]
    pub mode: (),
    #[tl(with = "tl_block_id_full")]
    pub id: BlockId,
    pub state_proof: Vec<u8>,
    pub config_proof: Vec<u8>,
}

#[derive(Debug, TlRead)]
#[tl(boxed, id = "liteServer.transactionList", scheme = "proto.tl")]
pub struct TransactionList {
    #[tl(with = "tl_vec_block_id_full")]
    pub block_ids: Vec<BlockId>,
    pub transactions: Vec<u8>,
}

#[derive(Debug, TlRead)]
#[tl(boxed, id = "liteServer.error", scheme = "proto.tl")]
pub struct Error {
    pub code: i32,
    #[tl(with = "tl_string")]
    pub message: String,
}

#[derive(Debug, TlWrite)]
#[tl(boxed, id = "liteServer.waitMasterchainSeqno", scheme = "proto.tl")]
pub struct WaitMasterchainSeqno {
    pub seqno: u32,
    pub timeout_ms: u32,
}

#[derive(TlWrite)]
pub struct WrappedQuery<T> {
    pub wait_masterchain_seqno: Option<WaitMasterchainSeqno>,
    pub query: T,
}

#[derive(Copy, Clone, Debug, TlRead, TlWrite)]
#[tl(size_hint = 68)]
pub struct ZeroStateIdExt {
    pub workchain: i32,
    pub root_hash: [u8; 32],
    pub file_hash: [u8; 32],
}

pub type HashRef<'tl> = &'tl [u8; 32];

pub mod rpc {
    use super::*;

    #[derive(Copy, Clone, TlWrite)]
    #[tl(boxed, id = "liteServer.sendMessage", scheme = "proto.tl")]
    pub struct SendMessage<'tl> {
        pub body: &'tl [u8],
    }

    #[derive(Copy, Clone, TlWrite)]
    #[tl(boxed, id = "liteServer.getVersion", scheme = "proto.tl")]
    pub struct GetVersion;

    #[derive(Copy, Clone, TlWrite)]
    #[tl(boxed, id = "liteServer.getMasterchainInfo", scheme = "proto.tl")]
    pub struct GetMasterchainInfo;

    #[derive(Copy, Clone, TlWrite)]
    #[tl(boxed, id = "liteServer.getBlock", scheme = "proto.tl")]
    pub struct GetBlock {
        #[tl(with = "tl_block_id_full")]
        pub id: BlockId,
    }

    #[derive(Copy, Clone, Debug, TlWrite)]
    #[tl(boxed, id = "liteServer.lookupBlock", scheme = "proto.tl")]
    pub struct LookupBlock {
        #[tl(flags)]
        pub mode: (),
        #[tl(with = "tl_block_id_short")]
        pub id: BlockIdShort,
        #[tl(flags_bit = "mode.0")]
        pub seqno: Option<()>,
        #[tl(flags_bit = "mode.1")]
        pub lt: Option<u64>,
        #[tl(flags_bit = "mode.2")]
        pub utime: Option<u32>,
    }

    #[derive(Clone, Debug, TlRead, TlWrite)]
    #[tl(boxed, id = "liteServer.getBlockProof", scheme = "proto.tl")]
    pub struct GetBlockProof {
        #[tl(flags)]
        pub mode: (),
        #[tl(with = "tl_block_id_full")]
        pub known_block: BlockId,
        #[tl(flags_bit = "mode.0", with = "tl_block_id_full")]
        pub target_block: Option<BlockId>,
    }

    #[derive(Clone, Debug, TlWrite)]
    #[tl(boxed, id = "liteServer.getConfigAll", scheme = "proto.tl")]
    pub struct GetConfigAll {
        #[tl(flags)]
        pub mode: (),
        #[tl(with = "tl_block_id_full")]
        pub id: BlockId,
        #[tl(flags_bit = "mode.4")]
        pub with_validator_set: Option<()>,
    }

    #[derive(Clone, Debug, TlWrite)]
    #[tl(boxed, id = "liteServer.getTransactions", scheme = "proto.tl")]
    pub struct GetTransactions {
        pub count: u32,
        #[tl(with = "tl_account_id")]
        pub account: StdAddr,
        pub lt: u64,
        pub hash: [u8; 32],
    }
}

mod tl_string {
    use tl_proto::{TlRead, TlResult};

    pub fn read(packet: &mut &[u8]) -> TlResult<String> {
        let bytes = <&[u8]>::read_from(packet)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

pub mod tl_block_id_short {
    use everscale_types::models::{BlockIdShort, ShardIdent};
    use tl_proto::{TlPacket, TlRead, TlResult, TlWrite};

    pub const SIZE_HINT: usize = 4 + 8 + 4;

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

pub mod tl_block_id_full {
    use everscale_types::models::BlockId;
    use everscale_types::prelude::HashBytes;
    use tl_proto::{TlPacket, TlRead, TlResult, TlWrite};

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

pub mod tl_vec_block_id_full {
    use tl_proto::{TlPacket, TlResult};

    use super::*;

    pub const fn size_hint(items: &[BlockId]) -> usize {
        4 + items.len() * tl_block_id_full::SIZE_HINT
    }

    pub fn write<P: TlPacket>(items: &[BlockId], packet: &mut P) {
        (items.len() as u32).write_to(packet);
        for item in items {
            tl_block_id_full::write(item, packet);
        }
    }

    pub fn read(packet: &mut &[u8]) -> TlResult<Vec<BlockId>> {
        let len = u32::read_from(packet)?;
        let mut res = Vec::with_capacity(len.min(64) as usize);
        for _ in 0..len {
            let block_id = tl_block_id_full::read(packet)?;
            res.push(block_id);
        }
        Ok(res)
    }
}

pub mod tl_account_id {
    use everscale_types::cell::HashBytes;
    use everscale_types::models::StdAddr;
    use tl_proto::{TlError, TlPacket, TlRead, TlResult, TlWrite};

    pub const fn size_hint(_: &StdAddr) -> usize {
        4 + 32
    }

    pub fn write<P: TlPacket>(addr: &StdAddr, packet: &mut P) {
        (addr.workchain as i32).write_to(packet);
        addr.address.0.write_to(packet);
    }

    pub fn read(packet: &mut &[u8]) -> TlResult<StdAddr> {
        let Ok::<i8, _>(workchain) = i32::read_from(packet)?.try_into() else {
            return Err(TlError::InvalidData);
        };
        let address = HashBytes(<[u8; 32]>::read_from(packet)?);

        Ok(StdAddr::new(workchain, address))
    }
}
