use anyhow::Result;
use everscale_types::error::Error;
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{BlockExtra, BlockInfo, CurrencyCollection, ShardHashes, ShardIdent};
use everscale_types::prelude::*;

/// Leaves only transaction hashes in block.
pub fn make_pruned_block<F>(block_root: Cell, mut on_tx: F) -> Result<Cell, Error>
where
    F: FnMut(Cell) -> Result<(), Error>,
{
    let usage_tree = UsageTree::new(UsageTreeMode::OnDataAccess);

    let tracked_root = usage_tree.track(&block_root);
    let raw_block = tracked_root.parse::<BlockShort>()?;

    // Include block extra for account blocks only.
    let extra = raw_block.extra.parse::<BlockExtra>()?;

    if extra.custom.is_some() {
        // Include block info for masterchain blocks.
        let info = raw_block.info.parse::<BlockInfo>()?;
        // Access `prev_ref` data to include it into the cell.
        info.prev_ref.data();

        if let Some(master_ref) = &info.master_ref {
            master_ref.inner().data();
        }
        if let Some(prev_vert_ref) = &info.prev_vert_ref {
            prev_vert_ref.inner().data();
        }
    }

    let account_blocks = extra.account_blocks.load()?;

    // Visit only items with transaction roots.
    for item in account_blocks.values() {
        let (_, account_block) = item?;

        // NOTE: Account block `transactions` dict is a new cell.
        let (transactions, _) = account_block.transactions.into_parts();
        let transactions = Dict::<u64, (CurrencyCollection, Cell)>::from_raw(
            transactions.into_root().map(|cell| usage_tree.track(&cell)),
        );

        for item in transactions.values() {
            let (_, tx) = item?;

            // Handle tx (without affecting the usage tree).
            let tx = Cell::untrack(tx);
            on_tx(tx)?;
        }
    }

    // Build block proof.
    let pruned_block = MerkleProof::create(block_root.as_ref(), usage_tree)
        .prune_big_cells(true)
        .build_raw_ext(Cell::empty_context())?;

    if pruned_block.hash(0) != block_root.repr_hash() {
        return Err(Error::InvalidData);
    }

    Ok(pruned_block)
}

pub fn make_mc_proof(block_root: Cell, shard: ShardIdent) -> Result<Cell, Error> {
    let usage_tree = UsageTree::new(UsageTreeMode::OnDataAccess);

    let tracked_root = usage_tree.track(&block_root);
    let raw_block = tracked_root.parse::<BlockShort>()?;

    // Block info is required for masterchain blocks to find the previous key block.
    // Only block info root cell is required (prev_ref is ignored).
    raw_block.info.parse::<BlockInfo>()?;

    // Access the required shard description.
    let extra = raw_block.extra.parse::<BlockExtraShort>()?;
    let custom = extra
        .custom
        .ok_or(Error::CellUnderflow)?
        .parse::<McBlockExtraShort>()?;

    let shard_hashes = custom
        .shard_hashes
        .get_workchain_shards(shard.workchain())?
        .ok_or(Error::CellUnderflow)?;

    let descr_root = find_shard_descr(shard_hashes.root(), shard.prefix())?;
    // Accessing data is required to mark the cell as visited.
    descr_root.data();

    // Build block proof.
    let pruned_block = MerkleProof::create(block_root.as_ref(), usage_tree)
        .prune_big_cells(true)
        .build_raw_ext(Cell::empty_context())?;

    if pruned_block.hash(0) != block_root.repr_hash() {
        return Err(Error::InvalidData);
    }

    Ok(pruned_block)
}

pub fn make_tx_proof(
    block_root: Cell,
    account: &HashBytes,
    lt: u64,
    include_info: bool,
) -> Result<Option<Cell>, Error> {
    let usage_tree = UsageTree::new(UsageTreeMode::OnDataAccess);

    let tracked_root = usage_tree.track(&block_root);
    let raw_block = tracked_root.parse::<BlockShort>()?;

    if include_info {
        let info = raw_block.info.parse::<BlockInfo>()?;
        // Touch `prev_ref` data to include it into the cell.
        info.prev_ref.data();
    }

    // Make a single branch with transaction.
    let extra = raw_block.extra.parse::<BlockExtraShort>()?;

    let account_blocks = extra.account_blocks.parse::<AccountBlocksShort>()?;
    let Some((_, account_block)) = account_blocks.get(account).ok().flatten() else {
        return Ok(None);
    };

    let (transactions, _) = account_block.transactions.into_parts();
    let transactions = Dict::<u64, (CurrencyCollection, Cell)>::from_raw(
        transactions.into_root().map(|cell| usage_tree.track(&cell)),
    );

    if transactions.get(lt).ok().flatten().is_none() {
        return Ok(None);
    };

    // Build block proof.
    let pruned_block = MerkleProof::create(block_root.as_ref(), usage_tree)
        .prune_big_cells(true)
        .build_raw_ext(Cell::empty_context())?;

    if pruned_block.hash(0) != block_root.repr_hash() {
        return Err(Error::InvalidData);
    }

    Ok(Some(pruned_block))
}

fn find_shard_descr(mut root: &'_ DynCell, mut prefix: u64) -> Result<&'_ DynCell, Error> {
    const HIGH_BIT: u64 = 1u64 << 63;

    debug_assert_ne!(prefix, 0);
    while prefix != HIGH_BIT {
        // Expect `bt_fork$1`.
        let mut cs = root.as_slice()?;
        if !cs.load_bit()? {
            return Err(Error::InvalidData);
        }

        // Get left (prefix bit 0) or right (prefix bit 1) branch.
        root = cs.get_reference((prefix & HIGH_BIT != 0) as u8)?;

        // Skip one prefix bit.
        prefix <<= 1;
    }

    // Root is now a `bt_leaf$0`.
    Ok(root)
}

#[derive(Load)]
#[tlb(tag = "#11ef55bb")]
struct BlockShort {
    _global_id: i32,
    info: Cell,
    _value_flow: Cell,
    _state_update: Cell,
    extra: Cell,
}

#[derive(Load)]
#[tlb(tag = "#4a33f6fc")]
struct BlockExtraShort {
    _in_msg_description: Cell,
    _out_msg_description: Cell,
    account_blocks: Cell,
    _rand_seed: HashBytes,
    _created_by: HashBytes,
    custom: Option<Cell>,
}

#[derive(Load)]
#[tlb(tag = "#cca5")]
struct McBlockExtraShort {
    _key_block: bool,
    shard_hashes: ShardHashes,
}

struct AccountBlockShort {
    transactions: AugDict<u64, CurrencyCollection, Cell>,
}

impl<'a> Load<'a> for AccountBlockShort {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        match slice.load_small_uint(4) {
            Ok(5) => {}
            Ok(_) => return Err(Error::InvalidTag),
            Err(e) => return Err(e),
        }

        slice.skip_first(256, 0)?;

        Ok(Self {
            transactions: AugDict::load_from_root_ext(slice, Cell::empty_context())?,
        })
    }
}

type AccountBlocksShort = AugDict<HashBytes, CurrencyCollection, AccountBlockShort>;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::Context;
    use everscale_types::boc::Boc;

    use super::*;

    #[test]
    #[ignore]
    fn prune_medium_block() -> Result<()> {
        let lt = 3141579000058;
        let account = "45c8b28ae239e122c292fc46fc3b852c6c629f25a91c5e07330e92cf298c7d81"
            .parse::<HashBytes>()?;

        // Read block.
        let block_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("res/block.boc");
        let block_root = Boc::decode(std::fs::read(block_path)?)?;

        // Remove everything except transaction hashes.
        println!("building pruned block");
        let pruned_block = make_pruned_block(block_root, |_tx_root| Ok(()))?;

        // Build a pruned block which contains a single branch to transaction.
        println!("building tx proof");
        let tx_proof = make_tx_proof(Cell::virtualize(pruned_block), &account, lt, false)?
            .context("tx not found in block")?;

        // Done.
        println!("serializing tx proof");
        let pruned = Boc::encode_base64(tx_proof);

        println!("PRUNED: {pruned}");
        Ok(())
    }
}
