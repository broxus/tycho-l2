use std::future::Future;
use std::time::Instant;

use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use everscale_types::error::Error;
use everscale_types::models::{ShardIdent, StdAddr};
use everscale_types::prelude::*;
use tycho_block_util::block::BlockStuff;
use tycho_util::sync::CancellationFlag;
use weedb::{
    rocksdb, Caches, MigrationError, OwnedSnapshot, Semver, VersionProvider, WeeDb, WeeDbRaw,
};

pub mod block;
pub mod tables;

pub struct ProofStorage {
    db: ProofDb,
    snapshot: ArcSwap<OwnedSnapshot>,
}

impl ProofStorage {
    pub async fn build_proof(&self, account: &StdAddr, lt: u64) -> Result<Option<Cell>> {
        fn decode_block(data: rocksdb::DBPinnableSlice<'_>) -> anyhow::Result<(HashBytes, Cell)> {
            let data = data.as_ref();
            let file_hash = HashBytes::from_slice(&data[..32]);
            let cell = Boc::decode(&data[32..])?;
            Ok((file_hash, cell))
        }

        let mut tx_key = [0u8; tables::Transactions::KEY_LEN];
        tx_key[0..8].copy_from_slice(&lt.to_be_bytes());
        tx_key[8] = account.workchain as u8;
        tx_key[9..41].copy_from_slice(account.address.as_slice());

        let mut block_key;
        let ref_by_mc_seqno;
        match self.db.transactions.get(tx_key)? {
            Some(value) => {
                let value = value.as_ref();
                block_key = <[u8; 32]>::try_from(&value[..13]).unwrap();
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

        let db = self.db.clone();
        let snapshot = self.snapshot.load_full();
        let cancelled = cancelled.clone();
        tokio::task::spawn_blocking(move || {
            check(&cancelled)?;

            let pruned_blocks_cf = &db.pruned_blocks.cf();
            let pivot_blocks_cf = &db.pivot_blocks.cf();

            let (tx_block_hash, block_with_tx) = snapshot
                .get_pinned_cf_opt(
                    pruned_blocks_cf,
                    block_key.as_slice(),
                    db.pruned_blocks.new_read_config(),
                )?
                .context("block not found")
                .and_then(decode_block)?;

            check(&cancelled)?;

            let tx_proof = block::make_tx_proof(block_with_tx, &account, lt, is_masterchain)?
                .context("tx not found in block")?;

            check(&cancelled)?;

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

                let mc = block::make_mc_proof(mc_block, shard)?;
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
                    let sc_block = snapshot
                        .get_pinned_cf_opt(
                            pivot_blocks_cf,
                            block_key.as_slice(),
                            db.pivot_blocks.new_read_config(),
                        )?
                        .context("pivot shard block not found")
                        .and_then(|data| Boc::decode(data).map_err(Into::into))?;

                    shard_proofs.push(sc_block);
                }

                shard_proofs.push(tx_proof);
            }

            check(&cancelled)?;

            let proof_chain = block::make_proof_chain(&file_hash, mc_proof, &shard_proofs)?;
            Ok::<_, anyhow::Error>(Some(proof_chain))
        })
        .await?
    }

    #[tracing::instrument(skip_all)]
    pub async fn store_block(&self, block: BlockStuff, ref_by_mc_seqno: u32) -> Result<()> {
        use everscale_types::boc::ser::BocHeader;

        fn encode_block(file_hash: &HashBytes, cell: Cell) -> Vec<u8> {
            let mut target = Vec::with_capacity(1024);
            target.extend_from_slice(file_hash.as_slice());
            BocHeader::<ahash::RandomState>::with_root(cell.as_ref()).encode(&mut target);
            target
        }

        let block_id = *block.id();
        let Ok::<i8, _>(workchain) = block_id.shard.workchain().try_into() else {
            return Ok(());
        };

        let cancelled = CancellationFlag::new();

        tracing::debug!("started");
        scopeguard::defer! {
            cancelled.cancel();
        }

        let span = tracing::Span::current();

        let db = self.db.clone();
        let cancelled = cancelled.clone();
        tokio::task::spawn_blocking(move || {
            let _span = span.enter();

            check(&cancelled)?;

            let is_masterchain = block_id.is_masterchain();

            let (pivot_tx, pivot_rx) = tokio::sync::oneshot::channel();
            rayon::spawn({
                let block = block.root_cell().clone();
                move || {
                    let started_at = Instant::now();
                    let res = block::make_pivot_block_proof(is_masterchain, block)
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
            let mut batch = rocksdb::WriteBatch::new();

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
            let pruned = block::make_pruned_block(block.root_cell().clone(), |account, lt| {
                if debounced.check() {
                    return Err(Error::Cancelled);
                }

                tx_key[0..8].copy_from_slice(&lt.to_be_bytes());
                tx_key[9..41].copy_from_slice(account.as_slice());
                batch.put_cf(transactions_cf, tx_key.as_slice(), tx_value.as_slice());
                Ok(())
            })
            .map(|cell| encode_block(&block_id.file_hash, cell))?;
            tracing::debug!(
                elapsed = %humantime::format_duration(started_at.elapsed()),
                "made pruned block"
            );

            check(&cancelled)?;

            batch.put_cf(pruned_blocks_cf, &tx_value[0..13], pruned);

            // Wait for the pivot block proof and put it to the batch.
            let pivot = pivot_rx.blocking_recv()??;
            batch.put_cf(pivot_blocks_cf, &tx_value[0..13], pivot);

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
