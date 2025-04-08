use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use arc_swap::{ArcSwap, ArcSwapAny, ArcSwapOption};
use bytesize::ByteSize;
use everscale_types::error::Error;
use everscale_types::models::{
    BlockId, BlockIdShort, BlockSignature, ShardIdent, StdAddr, ValidatorSet,
};
use everscale_types::prelude::*;
use proof_api_util::block::{self, TychoModels};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tycho_block_util::block::BlockStuff;
use tycho_storage::{FileDb, Storage};
use tycho_util::futures::JoinTask;
use tycho_util::serde_helpers;
use tycho_util::sync::CancellationFlag;
use tycho_util::time::now_sec;
use weedb::{
    rocksdb, Caches, MigrationError, OwnedSnapshot, Semver, Tables, VersionProvider, WeeDb,
    WeeDbRaw,
};

pub mod tables;

const PROOFS_SUBDIR: &str = "proofs";
const STORE_TIMINGS_STEP: u32 = 100; // Store timings every 100 mc blocks.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProofStorageConfig {
    /// Default: `4gb`.
    pub rocksdb_lru_capacity: ByteSize,
    /// Default: `false`.
    pub rocksdb_enable_metrics: bool,
    /// Default: `2 weeks`.
    #[serde(with = "serde_helpers::humantime")]
    pub min_proof_ttl: Duration,
    /// Default: `10 minutes`
    #[serde(with = "serde_helpers::humantime")]
    pub compaction_interval: Duration,
}

impl Default for ProofStorageConfig {
    #[inline]
    fn default() -> Self {
        Self {
            rocksdb_lru_capacity: ByteSize::gb(4),
            rocksdb_enable_metrics: false,
            min_proof_ttl: Duration::from_secs(14 * 86400),
            compaction_interval: Duration::from_secs(10 * 60),
        }
    }
}

#[derive(Clone)]
#[repr(transparent)]
pub struct ProofStorage {
    inner: Arc<Inner>,
}

struct Inner {
    db: ProofDb,
    snapshot: ArcSwap<OwnedSnapshot>,
    current_vset: ArcSwapOption<ValidatorSet>,
    min_proof_ttl_sec: u32,
    _compaction_handle: JoinTask<()>,
}

impl ProofStorage {
    pub async fn new(root: &FileDb, config: ProofStorageConfig) -> Result<Self> {
        const MAX_THREADS: usize = 8;

        let caches = weedb::Caches::with_capacity(config.rocksdb_lru_capacity.as_u64() as _);

        let threads = std::thread::available_parallelism()?.get().min(MAX_THREADS);
        let fdlimit = match fdlimit::raise_fd_limit() {
            // New fd limit
            Ok(fdlimit::Outcome::LimitRaised { to, .. }) => to,
            // Current soft limit
            _ => {
                rlimit::getrlimit(rlimit::Resource::NOFILE)
                    .unwrap_or((256, 0))
                    .0
            }
        };

        let db = ProofDb::builder(root.create_subdir(PROOFS_SUBDIR)?.path(), caches)
            .with_name(ProofDb::NAME)
            .with_metrics_enabled(config.rocksdb_enable_metrics)
            .with_options(|opts, _| {
                opts.set_paranoid_checks(false);

                // parallel compactions finishes faster - less write stalls
                opts.set_max_subcompactions(threads as u32 / 2);

                // io
                opts.set_max_open_files(fdlimit as i32);

                // logging
                opts.set_log_level(rocksdb::LogLevel::Info);
                opts.set_keep_log_file_num(2);
                opts.set_recycle_log_file_num(2);

                // cf
                opts.create_if_missing(true);
                opts.create_missing_column_families(true);

                // cpu
                opts.set_max_background_jobs(std::cmp::max((threads as i32) / 2, 2));
                opts.increase_parallelism(threads as i32);

                opts.set_allow_concurrent_memtable_write(false);
            })
            .build()?;

        db.apply_migrations().await?;

        trigger_compaction(&db).await?;

        let snapshot = db.owned_snapshot();

        let compaction_handle = JoinTask::new({
            let db = db.clone();
            let compaction_interval = config.compaction_interval;
            async move {
                let offset = rand::thread_rng().gen_range(Duration::ZERO..compaction_interval);
                tokio::time::sleep(offset).await;

                let mut interval = tokio::time::interval(compaction_interval);
                loop {
                    interval.tick().await;

                    if let Err(e) = trigger_compaction(&db).await {
                        tracing::error!("failed to trigger compaction: {e:?}");
                    }
                }
            }
        });

        Ok(Self {
            inner: Arc::new(Inner {
                db,
                snapshot: ArcSwap::new(Arc::new(snapshot)),
                current_vset: ArcSwapAny::default(),
                min_proof_ttl_sec: config
                    .min_proof_ttl
                    .as_secs()
                    .try_into()
                    .unwrap_or(u32::MAX),
                _compaction_handle: compaction_handle,
            }),
        })
    }

    #[allow(clippy::disallowed_methods)]
    pub async fn init(&self, storage: &Storage, init_block_id: &BlockId) -> Result<()> {
        let handles = storage.block_handle_storage();
        let states = storage.shard_state_storage();
        let blocks = storage.block_storage();

        // Init current vset.
        let current_vset = if init_block_id.seqno == 0 {
            // Load zerostate
            let zerostate = states
                .load_state(init_block_id)
                .await
                .context("failed to load zerostate")?;

            // Get current validator set from the state.
            zerostate
                .config_params()?
                .get_current_validator_set()
                .context("failed to get current validator set")
                .map(Arc::new)?
        } else {
            // Find the latest key block (relative to the `init_block_id`).
            let key_block_handle = handles
                .find_prev_key_block(init_block_id.seqno + 1)
                .context("no key block found")?;

            // Load proof.
            let block_proof = blocks
                .load_block_proof(&key_block_handle)
                .await
                .context("failed to load init key block proof")?;

            // Get current validator set from the proof.
            let (block, _) = block_proof.virtualize_block()?;
            let extra = block.extra.load()?;
            let custom = extra.load_custom()?.context("invalid key block")?;
            let config = custom.config.context("key block without config")?;

            config
                .get_current_validator_set()
                .context("failed to get current validator set")
                .map(Arc::new)?
        };

        self.set_current_vset(current_vset);

        // Done
        Ok(())
    }

    pub fn update_snapshot(&self) {
        let snapshot = self.inner.db.owned_snapshot();
        self.inner.snapshot.store(Arc::new(snapshot));
    }

    pub fn set_current_vset(&self, vset: Arc<ValidatorSet>) {
        self.inner.current_vset.store(Some(vset));
    }

    pub async fn build_proof(&self, account: &StdAddr, lt: u64) -> Result<Option<Cell>> {
        let this = self.inner.as_ref();

        let mut tx_key = [0u8; tables::Transactions::KEY_LEN];
        tx_key[0..8].copy_from_slice(&lt.to_be_bytes());
        tx_key[8] = account.workchain as u8;
        tx_key[9..41].copy_from_slice(account.address.as_slice());

        let mut block_key;
        let ref_by_mc_seqno;
        match this.db.transactions.get(tx_key)? {
            Some(value) => {
                let value = value.as_ref();
                block_key = <[u8; 13]>::try_from(&value[..13]).unwrap();
                ref_by_mc_seqno = u32::from_le_bytes(value[13..17].try_into().unwrap());
            }
            None => return Ok(None),
        };

        let tx_block_seqno = u32::from_be_bytes(block_key[9..13].try_into().unwrap());
        let shard = ShardIdent::new(
            block_key[0] as i8 as i32,
            u64::from_be_bytes(block_key[1..9].try_into().unwrap()),
        )
        .unwrap();

        let cancelled = CancellationFlag::new();
        scopeguard::defer! {
            cancelled.cancel();
        }

        let is_masterchain = account.is_masterchain();
        let account = account.address;

        let db = this.db.clone();
        let snapshot = this.snapshot.load_full();
        let cancelled = cancelled.clone();
        tokio::task::spawn_blocking(move || {
            check(&cancelled)?;

            let pruned_blocks_cf = &db.pruned_blocks.cf();
            let pivot_blocks_cf = &db.pivot_blocks.cf();
            let signatures_cf = &db.signatures.cf();

            let (tx_block_hash, block_with_tx) = snapshot
                .get_pinned_cf_opt(
                    pruned_blocks_cf,
                    block_key.as_slice(),
                    db.pruned_blocks.new_read_config(),
                )?
                .context("block not found")
                .and_then(decode_block)?;

            check(&cancelled)?;

            let tx_proof =
                block::make_tx_proof::<TychoModels>(block_with_tx, &account, lt, is_masterchain)?
                    .context("tx not found in block")?;

            check(&cancelled)?;

            // Get signatures.
            let (vset_utime_since, signatures) = snapshot
                .get_pinned_cf_opt(
                    signatures_cf,
                    ref_by_mc_seqno.to_be_bytes(),
                    db.signatures.new_read_config(),
                )?
                .context("signatures not found")
                .and_then(decode_signatures)?;

            // Get all required blocks.
            let file_hash;
            let mc_proof;
            let mut shard_proofs = Vec::new();
            if is_masterchain {
                // No shard blocks are required in addition to masterchain proof.
                file_hash = tx_block_hash;
                mc_proof = tx_proof;
            } else {
                // Get pivot mc block.
                let mut mc_block_key = [0; tables::PivotBlocks::KEY_LEN];
                mc_block_key[0] = -1i8 as u8;
                mc_block_key[1..9].copy_from_slice(&ShardIdent::MASTERCHAIN.prefix().to_be_bytes());
                mc_block_key[9..13].copy_from_slice(&ref_by_mc_seqno.to_be_bytes());
                let (mc_block_hash, mc_block) = snapshot
                    .get_pinned_cf_opt(
                        pivot_blocks_cf,
                        mc_block_key.as_slice(),
                        db.pivot_blocks.new_read_config(),
                    )?
                    .context("ref mc block not found")
                    .and_then(decode_block)?;

                let mc = block::make_mc_proof::<TychoModels>(mc_block, shard)?;
                file_hash = mc_block_hash;
                mc_proof = mc.root;

                anyhow::ensure!(
                    mc.latest_shard_seqno >= tx_block_seqno,
                    "stored masterchain block has some strange shard description"
                );

                // Iterate intermediate shard blocks in reverse order until the latest one.
                for seqno in (mc.latest_shard_seqno..tx_block_seqno).rev() {
                    check(&cancelled)?;

                    block_key[9..13].copy_from_slice(&seqno.to_be_bytes());
                    let (_, sc_block) = snapshot
                        .get_pinned_cf_opt(
                            pivot_blocks_cf,
                            block_key.as_slice(),
                            db.pivot_blocks.new_read_config(),
                        )?
                        .context("pivot shard block not found")
                        .and_then(decode_block)?;

                    shard_proofs.push(sc_block);
                }

                shard_proofs.push(tx_proof);
            }

            check(&cancelled)?;

            let proof_chain = block::make_proof_chain(
                &file_hash,
                mc_proof,
                &shard_proofs,
                vset_utime_since,
                signatures,
            )?;
            Ok::<_, anyhow::Error>(Some(proof_chain))
        })
        .await?
    }

    #[tracing::instrument(skip_all)]
    pub async fn store_block(
        &self,
        block: BlockStuff,
        signatures: Dict<u16, BlockSignature>,
        ref_by_mc_seqno: u32,
    ) -> Result<()> {
        let block_id = *block.id();
        let Ok::<i8, _>(workchain) = block_id.shard.workchain().try_into() else {
            return Ok(());
        };

        let cancelled = CancellationFlag::new();

        let now = now_sec();
        let min_proof_ttl = self.inner.min_proof_ttl_sec;

        let gen_utime = block.load_info()?.gen_utime;
        if now.saturating_sub(gen_utime) > min_proof_ttl {
            tracing::debug!(gen_utime, now, "skipped outdated block");
            return Ok(());
        }

        tracing::debug!("started");
        scopeguard::defer! {
            cancelled.cancel();
        }

        let span = tracing::Span::current();

        let vset = self
            .inner
            .current_vset
            .load_full()
            .context("no current vset found")?;

        let db = self.inner.db.clone();
        let cancelled = cancelled.clone();
        tokio::task::spawn_blocking(move || {
            let _span = span.enter();

            check(&cancelled)?;

            let is_masterchain = block_id.is_masterchain();

            let signatures_rx = if is_masterchain {
                let vset = vset.clone();
                let (signatures_tx, signatures_rx) = tokio::sync::oneshot::channel();
                rayon::spawn(move || {
                    let res = block::prepare_signatures(signatures.values(), &vset)
                        .map(|cell| encode_signatures(vset.utime_since, cell));

                    signatures_tx.send(res).ok();
                });
                Some(signatures_rx)
            } else {
                None
            };

            let (pivot_tx, pivot_rx) = tokio::sync::oneshot::channel();
            rayon::spawn({
                let block = block.root_cell().clone();
                move || {
                    let started_at = Instant::now();
                    let res = block::make_pivot_block_proof::<TychoModels>(is_masterchain, block)
                        .map(|cell| encode_block(&block_id.file_hash, cell));
                    tracing::debug!(
                        elapsed = %humantime::format_duration(started_at.elapsed()),
                        "made pivot block"
                    );

                    pivot_tx.send(res).ok();
                }
            });

            let pruned_blocks_cf = &db.pruned_blocks.cf();
            let pivot_blocks_cf = &db.pivot_blocks.cf();
            let transactions_cf = &db.transactions.cf();
            let signatures_cf = &db.signatures.cf();
            let timings_cf = &db.timings.cf();
            let mut batch = rocksdb::WriteBatch::new();

            // Add timings for masterchain blocks.
            let remove_bound_rx;
            if is_masterchain && block_id.seqno % STORE_TIMINGS_STEP == 0 {
                let remove_until = now.saturating_sub(min_proof_ttl);
                let db = db.clone();

                let (tx, rx) = tokio::sync::oneshot::channel();
                rayon::spawn(move || {
                    let res = find_outdated_bound(&db, remove_until);
                    tx.send(res).ok();
                });
                remove_bound_rx = Some(rx);

                batch.put_cf(
                    timings_cf,
                    gen_utime.to_be_bytes(),
                    block_id.seqno.to_le_bytes(),
                );
            } else {
                remove_bound_rx = None;
            }

            // Prepare tx key/value buffers.
            let mut tx_value = [0; tables::Transactions::VALUE_LEN];
            tx_value[0] = workchain as u8;
            tx_value[1..9].copy_from_slice(&block_id.shard.prefix().to_be_bytes());
            tx_value[9..13].copy_from_slice(&block_id.seqno.to_be_bytes());
            tx_value[13..17].copy_from_slice(&ref_by_mc_seqno.to_le_bytes());

            let mut tx_key = [0; tables::Transactions::KEY_LEN];
            tx_key[8] = workchain as u8;

            // Build pruned block and fill batch with new transactions.
            let started_at = Instant::now();
            let mut debounced = cancelled.debounce(100);
            let pruned = block::make_pruned_block::<TychoModels, _>(
                block.root_cell().clone(),
                |account, lt| {
                    if debounced.check() {
                        return Err(Error::Cancelled);
                    }

                    tx_key[0..8].copy_from_slice(&lt.to_be_bytes());
                    tx_key[9..41].copy_from_slice(account.as_slice());
                    batch.put_cf(transactions_cf, tx_key.as_slice(), tx_value.as_slice());
                    Ok(())
                },
            )
            .map(|cell| encode_block(&block_id.file_hash, cell))?;
            tracing::debug!(
                elapsed = %humantime::format_duration(started_at.elapsed()),
                "made pruned block"
            );

            check(&cancelled)?;

            batch.put_cf(pruned_blocks_cf, &tx_value[0..13], pruned);

            // Wait for signatures and put them to the batch.
            if let Some(signatures) = signatures_rx {
                debug_assert!(is_masterchain);
                let signatures = signatures.blocking_recv()??;
                batch.put_cf(signatures_cf, block_id.seqno.to_be_bytes(), signatures);
            }

            // Wait for the pivot block proof and put it to the batch.
            let pivot = pivot_rx.blocking_recv()??;
            batch.put_cf(pivot_blocks_cf, &tx_value[0..13], pivot);

            // Wait for bound to remove and put it to the batch.
            if let Some(bound) = remove_bound_rx {
                debug_assert!(is_masterchain);
                if let Some(bound) = bound.blocking_recv()?? {
                    batch.delete_range_cf(
                        timings_cf,
                        [0; tables::Timings::KEY_LEN],
                        bound.timings_key(),
                    );

                    batch.delete_range_cf(
                        transactions_cf,
                        [0; tables::Transactions::KEY_LEN],
                        bound.tx_key(),
                    );

                    const {
                        assert!(tables::PivotBlocks::KEY_LEN == tables::PrunedBlocks::KEY_LEN);
                    }

                    for (from_key, to_key) in bound.iter_block_keys() {
                        batch.delete_range_cf(pivot_blocks_cf, from_key, to_key);
                        batch.delete_range_cf(pruned_blocks_cf, from_key, to_key);
                    }
                }
            }

            // Write the result batch to rocksdb.
            let started_at = Instant::now();
            db.rocksdb()
                .write_opt(batch, db.transactions.write_config())
                .context("failed to write proofs batch")?;
            tracing::debug!(
                elapsed = %humantime::format_duration(started_at.elapsed()),
                "written new block"
            );

            Ok::<_, anyhow::Error>(())
        })
        .await?
    }
}

struct OutdatedBound {
    remove_until: u32,
    lt: u64,
    blocks: Vec<BlockIdShort>,
}

impl OutdatedBound {
    fn timings_key(&self) -> [u8; tables::Timings::KEY_LEN] {
        // Use next timestamp to remove everything before this key.
        (self.remove_until + 1).to_be_bytes()
    }

    fn tx_key(&self) -> [u8; tables::Transactions::KEY_LEN] {
        // Use next lt to remove everything before this key.
        let lt = self.lt + 1;

        let mut key = [0; tables::Transactions::KEY_LEN];
        key[0..8].copy_from_slice(&lt.to_be_bytes());
        key
    }

    fn iter_block_keys(&self) -> impl Iterator<Item = (BlockKey, BlockKey)> + '_ {
        self.blocks.iter().filter_map(|block_id| {
            let Ok::<i8, _>(workchain) = block_id.shard.workchain().try_into() else {
                return None;
            };

            // Use next seqno to remove everything before this key.
            let seqno = block_id.seqno + 1;

            let mut key = [0; tables::PivotBlocks::KEY_LEN];
            key[0] = workchain as u8;
            key[1..9].copy_from_slice(&block_id.shard.prefix().to_be_bytes());

            let from = key;
            key[9..13].copy_from_slice(&seqno.to_be_bytes());

            Some((from, key))
        })
    }
}

type BlockKey = [u8; tables::PivotBlocks::KEY_LEN];

fn find_outdated_bound(db: &ProofDb, remove_until: u32) -> Result<Option<OutdatedBound>> {
    let until_mc_seqno = {
        let mut iter = db.timings.raw_iterator();
        iter.seek_for_prev(remove_until.to_be_bytes());
        let Some(value) = iter.value() else {
            return Ok(None);
        };
        u32::from_le_bytes(value[..4].try_into().unwrap())
    };

    let mut mc_block_key = [0; tables::PivotBlocks::KEY_LEN];
    mc_block_key[0] = -1i8 as u8;
    mc_block_key[1..9].copy_from_slice(&ShardIdent::MASTERCHAIN.prefix().to_be_bytes());
    mc_block_key[9..13].copy_from_slice(&until_mc_seqno.to_be_bytes());

    let (_, block) = match db.pivot_blocks.get(mc_block_key)? {
        Some(data) => decode_block(data)?,
        None => return Ok(None),
    };

    let mut info = block::parse_latest_shard_blocks::<TychoModels>(block)?;
    info.shard_ids.push(BlockIdShort {
        shard: ShardIdent::MASTERCHAIN,
        seqno: until_mc_seqno,
    });

    Ok(Some(OutdatedBound {
        remove_until,
        lt: info.end_lt,
        blocks: info.shard_ids,
    }))
}

async fn trigger_compaction(db: &ProofDb) -> Result<()> {
    let cancelled = CancellationFlag::new();
    scopeguard::defer! {
        cancelled.cancel();
    }

    let span = tracing::Span::current();

    let db = db.clone();
    let cancelled = cancelled.clone();
    tokio::task::spawn_blocking(move || {
        let _span = span.enter();

        let mut compaction_options = rocksdb::CompactOptions::default();
        compaction_options.set_exclusive_manual_compaction(true);
        compaction_options
            .set_bottommost_level_compaction(rocksdb::BottommostLevelCompaction::ForceOptimized);

        for table in db.column_families() {
            check(&cancelled)?;

            tracing::info!(cf = table.name, "compaction started");

            let instant = Instant::now();
            let bound = Option::<[u8; 0]>::None;

            db.rocksdb()
                .compact_range_cf_opt(&table.cf, bound, bound, &compaction_options);

            tracing::info!(
                cf = table.name,
                elapsed_sec = %instant.elapsed().as_secs_f64(),
                "compaction finished"
            );
        }

        Ok::<_, anyhow::Error>(())
    })
    .await?
}

fn encode_block(file_hash: &HashBytes, cell: Cell) -> Vec<u8> {
    use everscale_types::boc::ser::BocHeader;

    let mut target = Vec::with_capacity(1024);
    target.extend_from_slice(file_hash.as_slice());
    BocHeader::<ahash::RandomState>::with_root(cell.as_ref()).encode(&mut target);
    target
}

fn decode_block(data: rocksdb::DBPinnableSlice<'_>) -> anyhow::Result<(HashBytes, Cell)> {
    let data = data.as_ref();
    let file_hash = HashBytes::from_slice(&data[..32]);
    let cell = Boc::decode(&data[32..])?;
    Ok((file_hash, cell))
}

fn encode_signatures(vset_utime_since: u32, cell: Cell) -> Vec<u8> {
    use everscale_types::boc::ser::BocHeader;

    let mut target = Vec::with_capacity(256);
    target.extend_from_slice(&vset_utime_since.to_le_bytes());
    BocHeader::<ahash::RandomState>::with_root(cell.as_ref()).encode(&mut target);
    target
}

fn decode_signatures(data: rocksdb::DBPinnableSlice<'_>) -> anyhow::Result<(u32, Cell)> {
    let data = data.as_ref();
    let utime_since = u32::from_le_bytes(data[..4].try_into().unwrap());
    let cell = Boc::decode(&data[4..])?;
    Ok((utime_since, cell))
}

pub type ProofDb = WeeDb<ProofTables>;

trait ProofDbExt: Sized {
    const NAME: &'static str;
    const VERSION: Semver;

    fn register_migrations(
        migrations: &mut Migrations<Self>,
        cancelled: CancellationFlag,
    ) -> Result<(), MigrationError>;

    fn apply_migrations(&self) -> impl Future<Output = Result<(), MigrationError>> + Send;
}

impl ProofDbExt for ProofDb {
    const NAME: &'static str = "proofs";
    const VERSION: Semver = [0, 0, 1];

    fn register_migrations(
        _migrations: &mut Migrations<Self>,
        _cancelled: CancellationFlag,
    ) -> Result<(), MigrationError> {
        // TODO: Add migrations here.
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(db = Self::NAME))]
    async fn apply_migrations(&self) -> Result<(), MigrationError> {
        let cancelled = CancellationFlag::new();

        tracing::info!("started");
        scopeguard::defer! {
            cancelled.cancel();
        }

        let span = tracing::Span::current();

        let this = self.clone();
        let cancelled = cancelled.clone();
        tokio::task::spawn_blocking(move || {
            let _span = span.enter();

            let guard = scopeguard::guard((), |_| {
                tracing::warn!("cancelled");
            });

            let mut migrations = Migrations::<Self>::with_target_version_and_provider(
                Self::VERSION,
                StateVersionProvider {
                    db_name: Self::NAME,
                },
            );

            Self::register_migrations(&mut migrations, cancelled)?;

            this.apply(migrations)?;

            scopeguard::ScopeGuard::into_inner(guard);
            tracing::info!("finished");
            Ok(())
        })
        .await
        .map_err(|e| MigrationError::Custom(e.into()))?
    }
}

weedb::tables! {
    pub struct ProofTables<Caches> {
        state: tables::State,
        pruned_blocks: tables::PrunedBlocks,
        pivot_blocks: tables::PivotBlocks,
        transactions: tables::Transactions,
        signatures: tables::Signatures,
        timings: tables::Timings,
    }
}

type Migrations<D> = weedb::Migrations<StateVersionProvider, D>;

struct StateVersionProvider {
    db_name: &'static str,
}

impl StateVersionProvider {
    const DB_NAME_KEY: &'static [u8] = b"__db_name";
    const DB_VERSION_KEY: &'static [u8] = b"__db_version";
}

impl VersionProvider for StateVersionProvider {
    fn get_version(&self, db: &WeeDbRaw) -> Result<Option<Semver>, MigrationError> {
        let state = db.instantiate_table::<tables::State>();

        if let Some(db_name) = state.get(Self::DB_NAME_KEY)? {
            if db_name.as_ref() != self.db_name.as_bytes() {
                return Err(MigrationError::Custom(
                    format!(
                        "expected db name: {}, got: {}",
                        self.db_name,
                        String::from_utf8_lossy(db_name.as_ref())
                    )
                    .into(),
                ));
            }
        }

        let value = state.get(Self::DB_VERSION_KEY)?;
        match value {
            Some(version) => {
                let slice = version.as_ref();
                slice
                    .try_into()
                    .map_err(|_e| MigrationError::InvalidDbVersion)
                    .map(Some)
            }
            None => Ok(None),
        }
    }

    fn set_version(&self, db: &WeeDbRaw, version: Semver) -> Result<(), MigrationError> {
        let state = db.instantiate_table::<tables::State>();

        state.insert(Self::DB_NAME_KEY, self.db_name.as_bytes())?;
        state.insert(Self::DB_VERSION_KEY, version)?;
        Ok(())
    }
}

fn check(cancelled: &CancellationFlag) -> Result<()> {
    if cancelled.check() {
        Err(Error::Cancelled.into())
    } else {
        Ok(())
    }
}
