use tl_proto::{TlRead, TlWrite};

// ----- Types ----- //

#[derive(TlWrite)]
#[tl(boxed, id = "adnl.message.query", scheme = "proto.tl")]
pub struct AdnlMessageQuery<'tl, T> {
    #[tl(size_hint = 32)]
    pub query_id: HashRef<'tl>,
    #[tl(with = "struct_as_bytes")]
    pub query: LiteQuery<T>,
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
    #[tl(with = "struct_as_bytes")]
    pub wrapped_request: WrappedQuery<T>,
}

#[derive(Debug, TlRead)]
#[tl(boxed, id = "liteServer.masterchainInfo", scheme = "proto.tl")]
pub struct MasterchainInfo {
    pub last: BlockIdExt,
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
    pub id: BlockIdExt,
    pub data: Vec<u8>,
}

#[derive(Debug, TlRead)]
#[tl(boxed, id = "liteServer.blockHeader", scheme = "proto.tl")]
pub struct BlockHeader {
    pub id: BlockIdExt,
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

#[derive(Copy, Clone, Debug, TlRead, TlWrite)]
#[tl(size_hint = 16)]
pub struct BlockId {
    pub workchain: i32,
    pub shard: u64,
    pub seqno: u32,
}

impl From<everscale_types::models::BlockIdShort> for BlockId {
    fn from(item: everscale_types::models::BlockIdShort) -> Self {
        BlockId {
            workchain: item.shard.workchain(),
            shard: item.shard.prefix(),
            seqno: item.seqno,
        }
    }
}

#[derive(Copy, Clone, Debug, TlRead, TlWrite)]
#[tl(size_hint = 80)]
pub struct BlockIdExt {
    pub workchain: i32,
    pub shard: u64,
    pub seqno: u32,
    pub root_hash: [u8; 32],
    pub file_hash: [u8; 32],
}

impl From<BlockIdExt> for everscale_types::models::BlockId {
    fn from(item: BlockIdExt) -> Self {
        everscale_types::models::BlockId {
            shard: everscale_types::models::ShardIdent::new(item.workchain, item.shard)
                .unwrap_or_default(),
            seqno: item.seqno,
            root_hash: item.root_hash.into(),
            file_hash: item.file_hash.into(),
        }
    }
}

impl From<everscale_types::models::BlockId> for BlockIdExt {
    fn from(item: everscale_types::models::BlockId) -> Self {
        BlockIdExt {
            workchain: item.shard.workchain(),
            shard: item.shard.prefix(),
            seqno: item.seqno,
            root_hash: item.root_hash.0,
            file_hash: item.file_hash.0,
        }
    }
}

pub type HashRef<'tl> = &'tl [u8; 32];

// ----- Functions ----- //

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
    pub id: BlockIdExt,
}

#[derive(Copy, Clone, Debug, TlRead, TlWrite)]
#[tl(boxed, id = "liteServer.lookupBlock", scheme = "proto.tl")]
pub struct LookupBlock {
    #[tl(flags)]
    pub mode: (),
    pub id: BlockId,
    #[tl(flags_bit = "mode.0")]
    pub seqno: Option<()>,
    #[tl(flags_bit = "mode.1")]
    pub lt: Option<u64>,
    #[tl(flags_bit = "mode.2")]
    pub utime: Option<u32>,
}

mod tl_string {
    use tl_proto::{TlRead, TlResult};

    pub fn read(packet: &mut &[u8]) -> TlResult<String> {
        let bytes = <&[u8]>::read_from(packet)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

mod struct_as_bytes {
    use tl_proto::{TlPacket, TlWrite};

    pub fn size_hint<T: TlWrite>(v: &T) -> usize {
        tl_proto::serialize(v).len()
    }

    pub fn write<P: TlPacket, T: TlWrite>(v: &T, packet: &mut P) {
        tl_proto::serialize(v).write_to(packet);
    }
}
