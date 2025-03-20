use tl_proto::{TlRead, TlWrite};

// ----- Types ----- //

#[derive(TlWrite)]
#[tl(boxed, id = "adnl.message.query", scheme = "proto.tl")]
pub struct AdnlMessageQuery<'tl, T> {
    #[tl(size_hint = 32)]
    pub query_id: &'tl [u8; 32],
    #[tl(with = "struct_as_bytes")]
    pub query: LiteQuery<T>,
}

#[derive(Copy, Clone, TlRead)]
#[tl(boxed, id = "adnl.message.answer", scheme = "proto.tl")]
pub struct AdnlMessageAnswer<'tl> {
    #[tl(size_hint = 32)]
    pub query_id: &'tl [u8; 32],
    pub data: &'tl [u8],
}

#[derive(TlWrite)]
#[tl(boxed, id = "liteServer.query", scheme = "proto.tl")]
pub struct LiteQuery<T> {
    #[tl(with = "struct_as_bytes")]
    pub wrapped_request: WrappedQuery<T>,
}

#[derive(Debug, TlRead)]
pub struct MasterchainInfo {
    pub last: BlockIdExt,
    pub state_root_hash: [u8; 32],
    pub init: ZeroStateIdExt,
}

#[derive(Clone, Copy, Debug, TlRead)]
pub struct Version {
    pub mode: u32,
    pub version: u32,
    pub capabilities: u64,
    pub now: u32,
}

#[derive(Clone, Copy, Debug, TlRead)]
pub struct SendMsgStatus {
    pub status: u32,
}

#[derive(Debug, TlRead)]
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

pub type HashRef<'tl> = &'tl [u8; 32];

#[derive(Copy, Clone, Debug, TlRead, TlWrite)]
#[tl(size_hint = 80)]
pub struct BlockIdExt {
    pub workchain: i32,
    pub shard: u64,
    pub seqno: u32,
    pub root_hash: [u8; 32],
    pub file_hash: [u8; 32],
}

#[derive(Copy, Clone, Debug, TlRead, TlWrite)]
#[tl(size_hint = 68)]
pub struct ZeroStateIdExt {
    pub workchain: i32,
    #[tl(size_hint = 32)]
    pub root_hash: [u8; 32],
    #[tl(size_hint = 32)]
    pub file_hash: [u8; 32],
}

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

// ----- Responses ----- //

#[derive(TlRead)]
#[tl(boxed)]
pub enum Response {
    #[tl(id = 0x5a0491e5)]
    Version(Version),

    #[tl(id = 0x85832881)]
    MasterchainInfo(MasterchainInfo),

    #[tl(id = 0x3950e597)]
    SendMsgStatus(SendMsgStatus),

    #[tl(id = 0xbba9e148)]
    Error(Error),
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
