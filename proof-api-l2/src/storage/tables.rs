use weedb::rocksdb::{BlockBasedOptions, DBCompressionType, DataBlockIndexType, Options};
use weedb::{Caches, ColumnFamily, ColumnFamilyOptions};

// took from
// https://github.com/tikv/tikv/blob/d60c7fb6f3657dc5f3c83b0e3fc6ac75636e1a48/src/config/mod.rs#L170
// todo: need to benchmark and update if it's not optimal
const DEFAULT_MIN_BLOB_SIZE: u64 = 1024 * 32;

/// Stores generic node parameters
/// - Key: `...`
/// - Value: `...`
pub struct State;

impl ColumnFamily for State {
    const NAME: &'static str = "state";
}

impl ColumnFamilyOptions<Caches> for State {
    fn options(opts: &mut Options, caches: &mut Caches) {
        default_block_based_table_factory(opts, caches);

        opts.set_optimize_filters_for_hits(true);
        optimize_for_point_lookup(opts, caches);
    }
}

/// Stores the least possible proof.
/// - Key: `workchain: i8, shard: u64 (BE), seqno: u32 (BE)`
/// - Value: `file_hash: uint256, ...BOC`
pub struct PivotBlocks;

impl PivotBlocks {
    pub const KEY_LEN: usize = 1 + 8 + 4;
}

impl ColumnFamily for PivotBlocks {
    const NAME: &'static str = "pivot_blocks";
}

impl ColumnFamilyOptions<Caches> for PivotBlocks {
    fn options(opts: &mut Options, ctx: &mut Caches) {
        zstd_block_based_table_factory(opts, ctx);
        opts.set_compression_type(DBCompressionType::Zstd);
        with_blob_db(opts, DEFAULT_MIN_BLOB_SIZE, DBCompressionType::Zstd);
    }
}

/// Stores pruned blocks with transactions.
/// - Key: `workchain: i8, shard: u64 (BE), seqno: u32 (BE)`
/// - Value: `file_hash: uint256, ...BOC`
pub struct PrunedBlocks;

impl PrunedBlocks {
    pub const KEY_LEN: usize = 1 + 8 + 4;
}

impl ColumnFamily for PrunedBlocks {
    const NAME: &'static str = "pruned_blocks";
}

impl ColumnFamilyOptions<Caches> for PrunedBlocks {
    fn options(opts: &mut Options, ctx: &mut Caches) {
        zstd_block_based_table_factory(opts, ctx);
        opts.set_compression_type(DBCompressionType::Zstd);
        with_blob_db(opts, DEFAULT_MIN_BLOB_SIZE, DBCompressionType::Zstd);
    }
}

/// Stores transactions index.
/// - Key: `lt: u64 (BE), workchain: i8, account: [u8; 32]`
/// - Value: `workchain: i8, shard: u64 (BE), seqno: u32 (BE), ref_by_mc_seqno: u32 (LE)`
pub struct Transactions;

impl Transactions {
    pub const KEY_LEN: usize = 8 + 1 + 32;
    pub const VALUE_LEN: usize = PrunedBlocks::KEY_LEN + 4;
}

impl ColumnFamily for Transactions {
    const NAME: &'static str = "transactions";
}

impl ColumnFamilyOptions<Caches> for Transactions {
    fn options(opts: &mut Options, ctx: &mut Caches) {
        zstd_block_based_table_factory(opts, ctx);
        opts.set_compression_type(DBCompressionType::Zstd);
        with_blob_db(opts, DEFAULT_MIN_BLOB_SIZE, DBCompressionType::Zstd);
    }
}

/// Stores info for the start bound of the GC.
///
/// - Key: `created_at: u32 (BE)`
/// - Value: `mc_seqno: u32 (LE)`
pub struct Timings;

impl Timings {
    pub const KEY_LEN: usize = 4;
}

impl ColumnFamily for Timings {
    const NAME: &'static str = "block_timings";
}

impl ColumnFamilyOptions<Caches> for Timings {
    fn options(opts: &mut Options, ctx: &mut Caches) {
        default_block_based_table_factory(opts, ctx);
        opts.set_compression_type(DBCompressionType::Zstd);
    }
}

/// Stores block proof signatures.
/// - Key: `mc_seqno: u32`
/// - Value: `utime_since: u32, signatures: ...BOC`
pub struct Signatures;

impl Signatures {
    pub const KEY_LEN: usize = 4;
}

impl ColumnFamily for Signatures {
    const NAME: &'static str = "signatures";
}

impl ColumnFamilyOptions<Caches> for Signatures {
    fn options(opts: &mut Options, ctx: &mut Caches) {
        zstd_block_based_table_factory(opts, ctx);
        opts.set_compression_type(DBCompressionType::Zstd);
        with_blob_db(opts, DEFAULT_MIN_BLOB_SIZE, DBCompressionType::Zstd);
    }
}

fn default_block_based_table_factory(opts: &mut Options, caches: &Caches) {
    opts.set_level_compaction_dynamic_level_bytes(true);
    let mut block_factory = BlockBasedOptions::default();
    block_factory.set_block_cache(&caches.block_cache);
    block_factory.set_format_version(5);
    opts.set_block_based_table_factory(&block_factory);
}

// setting our shared cache instead of individual caches for each cf
fn optimize_for_point_lookup(opts: &mut Options, caches: &Caches) {
    //     https://github.com/facebook/rocksdb/blob/81aeb15988e43c49952c795e32e5c8b224793589/options/options.cc
    //     BlockBasedTableOptions block_based_options;
    //     block_based_options.data_block_index_type =
    //         BlockBasedTableOptions::kDataBlockBinaryAndHash;
    //     block_based_options.data_block_hash_table_util_ratio = 0.75;
    //     block_based_options.filter_policy.reset(NewBloomFilterPolicy(10));
    //     block_based_options.block_cache =
    //         NewLRUCache(static_cast<size_t>(block_cache_size_mb * 1024 * 1024));
    //     table_factory.reset(new BlockBasedTableFactory(block_based_options));
    //     memtable_prefix_bloom_size_ratio = 0.02;
    //     memtable_whole_key_filtering = true;
    //
    let mut block_factory = BlockBasedOptions::default();
    block_factory.set_data_block_index_type(DataBlockIndexType::BinaryAndHash);
    block_factory.set_data_block_hash_ratio(0.75);
    block_factory.set_bloom_filter(10.0, false);
    block_factory.set_block_cache(&caches.block_cache);
    opts.set_block_based_table_factory(&block_factory);

    opts.set_memtable_prefix_bloom_ratio(0.02);
    opts.set_memtable_whole_key_filtering(true);
}

fn zstd_block_based_table_factory(opts: &mut Options, caches: &Caches) {
    let mut block_factory = BlockBasedOptions::default();
    block_factory.set_block_cache(&caches.block_cache);
    opts.set_block_based_table_factory(&block_factory);
    opts.set_compression_type(DBCompressionType::Zstd);
}

fn with_blob_db(opts: &mut Options, min_value_size: u64, compression_type: DBCompressionType) {
    opts.set_enable_blob_files(true);
    opts.set_enable_blob_gc(true);

    opts.set_min_blob_size(min_value_size);
    opts.set_blob_compression_type(compression_type);
}
