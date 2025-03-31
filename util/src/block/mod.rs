use ahash::HashMap;
use everscale_types::cell::Lazy;
use everscale_types::error::Error;
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{
    Block, BlockId, BlockIdShort, BlockSignature, BlockchainConfig, CurrencyCollection, ShardIdent,
    ValidatorBaseInfo, ValidatorSet,
};
use everscale_types::prelude::*;

pub use self::ton::TonModels;
pub use self::tycho::TychoModels;

pub mod ton;
pub mod tycho;

// === Traits ===

pub trait BlockchainModels {
    type Block: BlockchainBlock;
    type BlockSignatures: BlockchainBlockSignatures;
}

pub trait BlockchainBlock: for<'a> Load<'a> {
    type Info: BlockchainBlockInfo;
    type Extra: BlockchainBlockExtra;

    fn load_info(&self) -> Result<Self::Info, Error>;
    fn load_info_raw(&self) -> Result<Cell, Error>;

    fn load_extra(&self) -> Result<Self::Extra, Error>;
}

pub trait BlockchainBlockInfo: for<'a> Load<'a> {
    fn is_key_block(&self) -> bool;
    fn end_lt(&self) -> u64;
    fn prev_ref(&self) -> &Cell;
}

pub trait BlockchainBlockExtra: for<'a> Load<'a> {
    type McExtra: BlockchainBlockMcExtra;

    fn load_account_blocks(&self) -> Result<AccountBlocksShort, Error>;

    fn has_custom(&self) -> bool;
    fn load_custom(&self) -> Result<Option<Self::McExtra>, Error>;
}

pub trait BlockchainBlockMcExtra: for<'a> Load<'a> {
    fn load_top_shard_block_ids(&self) -> Result<Vec<BlockIdShort>, Error>;
    fn find_shard_seqno(&self, shard_ident: ShardIdent) -> Result<u32, Error>;
    fn visit_all_shard_hashes(&self) -> Result<(), Error>;
    fn config(&self) -> Option<&BlockchainConfig>;
}

pub trait BlockchainBlockSignatures: for<'a> Load<'a> {
    fn validator_info(&self) -> ValidatorBaseInfo;
    fn signature_count(&self) -> u32;
    fn total_weight(&self) -> u64;
    fn signatures(&self) -> Dict<u16, BlockSignature>;
}

pub struct BaseBlockProof<S> {
    pub proof_for: BlockId,
    pub root: Cell,
    pub signatures: Option<Lazy<S>>,
}

impl<S> BaseBlockProof<S> {
    const TAG: u8 = 0xc3;

    pub fn load_signatures<'a>(&'a self) -> Result<Option<S>, Error>
    where
        S: Load<'a>,
    {
        match &self.signatures {
            Some(s) => s.load().map(Some),
            None => Ok(None),
        }
    }
}

impl<'a, S> Load<'a> for BaseBlockProof<S> {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        match slice.load_u8() {
            Ok(Self::TAG) => {}
            Ok(_) => return Err(Error::InvalidTag),
            Err(e) => return Err(e),
        }

        Ok(Self {
            proof_for: BlockId::load_from(slice)?,
            root: slice.load_reference_cloned()?,
            signatures: if slice.load_bit()? {
                let cell = slice.load_reference_cloned()?;
                Lazy::from_raw(cell).map(Some)?
            } else {
                None
            },
        })
    }
}

pub struct AccountBlockShort {
    pub account: HashBytes,
    pub transactions: AugDict<u64, CurrencyCollection, Cell>,
}

impl<'a> Load<'a> for AccountBlockShort {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        match slice.load_small_uint(4) {
            Ok(5) => {}
            Ok(_) => return Err(Error::InvalidTag),
            Err(e) => return Err(e),
        }

        Ok(Self {
            account: slice.load_u256()?,
            transactions: AugDict::load_from_root_ext(slice, Cell::empty_context())?,
        })
    }
}

pub type AccountBlocksShort = AugDict<HashBytes, CurrencyCollection, AccountBlockShort>;

// === Proff stuff ===

pub struct McBlockBoundInfo {
    pub end_lt: u64,
    pub shard_ids: Vec<BlockIdShort>,
}

/// Parses all shard descriptions from masterchain block.
///
/// Input: pivot mc block.
pub fn parse_latest_shard_blocks<M>(block_root: Cell) -> Result<McBlockBoundInfo, Error>
where
    M: BlockchainModels,
{
    let block = block_root.parse::<M::Block>()?;
    let info = block.load_info()?;

    let extra = block.load_extra()?;
    let custom = extra.load_custom()?.ok_or(Error::CellUnderflow)?;

    let shard_ids = custom.load_top_shard_block_ids()?;

    Ok(McBlockBoundInfo {
        end_lt: info.end_lt(),
        shard_ids,
    })
}

/// Prepares a signatures dict with validator indices as keys.
pub fn prepare_signatures<I>(signatures: I, vset: &ValidatorSet) -> Result<Cell, Error>
where
    I: IntoIterator<Item = Result<BlockSignature, Error>>,
{
    struct PlainSignature([u8; 64]);

    impl Store for PlainSignature {
        #[inline]
        fn store_into(&self, b: &mut CellBuilder, _: &dyn CellContext) -> Result<(), Error> {
            b.store_raw(&self.0, 512)
        }
    }

    let mut block_signatures = HashMap::default();
    for entry in signatures {
        let entry = entry?;
        let res = block_signatures.insert(entry.node_id_short, entry.signature);
        if res.is_some() {
            return Err(Error::InvalidData);
        }
    }

    let mut result = Vec::with_capacity(block_signatures.len());
    for (i, desc) in vset.list.iter().enumerate() {
        let key_hash = tl_proto::hash(everscale_crypto::tl::PublicKey::Ed25519 {
            key: desc.public_key.as_array(),
        });
        let Some(signature) = block_signatures.remove(HashBytes::wrap(&key_hash)) else {
            continue;
        };
        result.push((i as u16, PlainSignature(signature.0)));
    }

    if !block_signatures.is_empty() {
        return Err(Error::InvalidData);
    }

    let signatures = Dict::try_from_sorted_slice(&result)?;
    signatures.into_root().ok_or(Error::EmptyProof)
}

pub fn check_signatures<I>(
    block_id: &BlockId,
    signatures: I,
    vset: &ValidatorSet,
) -> Result<(), Error>
where
    I: IntoIterator<Item = Result<BlockSignature, Error>>,
{
    // Collect signatures into a map.
    let mut signatures = signatures
        .into_iter()
        .map(|x| x.map(|item| (item.node_id_short, item.signature)))
        .collect::<Result<HashMap<_, _>, _>>()?;

    let to_sign = Block::build_data_for_sign(block_id);

    let mut weight = 0u64;
    for node in &vset.list {
        let node_id_short = tl_proto::hash(everscale_crypto::tl::PublicKey::Ed25519 {
            key: node.public_key.as_ref(),
        });
        let node_id_short = HashBytes::wrap(&node_id_short);

        if let Some(signature) = signatures.remove(node_id_short) {
            if !node.verify_signature(&to_sign, &signature) {
                return Err(Error::InvalidSignature);
            }

            weight = weight.checked_add(node.weight).ok_or(Error::IntOverflow)?;
        }
    }

    // All signatures must be used.
    if !signatures.is_empty() {
        return Err(Error::InvalidData);
    }

    // Check that signature weight is enough.
    match (weight.checked_mul(3), vset.total_weight.checked_mul(2)) {
        (Some(weight_x3), Some(total_weight_x2)) => {
            if weight_x3 > total_weight_x2 {
                Ok(())
            } else {
                Err(Error::InvalidData)
            }
        }
        _ => Err(Error::IntOverflow),
    }
}

/// Build merkle proof cell which contains a proof chain in its root.
pub fn make_proof_chain(
    mc_file_hash: &HashBytes,
    mc_block: Cell,
    shard_blocks: &[Cell],
    vset_utime_since: u32,
    signatures: Cell,
) -> Result<Cell, Error> {
    let mut b = CellBuilder::new();
    b.store_u256(mc_file_hash)?;
    b.store_u32(vset_utime_since)?;
    b.store_reference(mc_block)?;
    b.store_reference(signatures)?;

    let mut iter = shard_blocks.iter();
    if let Some(sc_block) = iter.next() {
        b.store_reference(sc_block.clone())?;

        let mut iter = iter.rev();

        let remaining = iter.len();
        let mut child = if remaining % 3 != 0 {
            let mut b = CellBuilder::new();
            for cell in iter.by_ref().take(remaining % 3) {
                b.store_reference(cell.clone())?;
            }
            Some(b.build()?)
        } else {
            None
        };

        for _ in 0..(remaining / 3) {
            let sc1 = iter.next().unwrap();
            let sc2 = iter.next().unwrap();
            let sc3 = iter.next().unwrap();

            let mut b = CellBuilder::new();
            b.store_reference(sc3.clone())?;
            b.store_reference(sc2.clone())?;
            b.store_reference(sc1.clone())?;
            if let Some(child) = child.take() {
                b.store_reference(child)?;
            }
            child = Some(b.build()?);
        }

        if let Some(child) = child {
            b.store_reference(child)?;
        }
    }

    let cell = b.build()?;
    CellBuilder::build_from(MerkleProof {
        hash: *cell.hash(0),
        depth: cell.depth(0),
        cell,
    })
}

/// Leaves only transaction hashes in block.
///
/// Input: full block.
pub fn make_pruned_block<M, F>(block_root: Cell, mut on_tx: F) -> Result<Cell, Error>
where
    M: BlockchainModels,
    for<'a> F: FnMut(&'a HashBytes, u64) -> Result<(), Error>,
{
    let usage_tree = UsageTree::new(UsageTreeMode::OnDataAccess);

    let tracked_root = usage_tree.track(&block_root);
    let raw_block = tracked_root.parse::<M::Block>()?;

    // Include block extra for account blocks only.
    let extra = raw_block.load_extra()?;

    if extra.has_custom() {
        // Include full block info for masterchain blocks.
        let info = raw_block.load_info_raw()?;
        info.touch_recursive();
    }

    let account_blocks = extra.load_account_blocks()?;

    // Visit only items with transaction roots.
    for item in account_blocks.values() {
        let (_, account_block) = item?;

        // NOTE: Account block `transactions` dict is a new cell.
        let (transactions, _) = account_block.transactions.into_parts();
        let transactions = Dict::<u64, (CurrencyCollection, Cell)>::from_raw(
            transactions.into_root().map(|cell| usage_tree.track(&cell)),
        );

        for item in transactions.iter() {
            let (lt, _) = item?;

            // Handle tx.
            on_tx(&account_block.account, lt)?;
        }
    }

    // Build block proof.
    let pruned_block = MerkleProof::create(block_root.as_ref(), usage_tree)
        .prune_big_cells(true)
        .build_raw_ext(Cell::empty_context())?;

    if pruned_block.hash(0) != block_root.hash(0) {
        return Err(Error::InvalidData);
    }

    Ok(pruned_block)
}

/// Creates a small proof which can be used to build proof chains.
///
/// Input: full block.
pub fn make_pivot_block_proof<M>(is_masterchain: bool, block_root: Cell) -> Result<Cell, Error>
where
    M: BlockchainModels,
{
    let usage_tree = UsageTree::new(UsageTreeMode::OnDataAccess);

    let tracked_root = usage_tree.track(&block_root);
    let raw_block = tracked_root.parse::<M::Block>()?;

    if is_masterchain {
        // Include full block info for masterchain blocks.
        raw_block.load_info_raw()?.touch_recursive();

        // Include shard descriptions for all shards.
        let extra = raw_block.load_extra()?;
        let custom = extra.load_custom()?.ok_or(Error::CellUnderflow)?;

        custom.visit_all_shard_hashes()?;
    } else {
        // Include only prev block ref for shard blocks.
        let info = raw_block.load_info()?;
        info.prev_ref().data();
    }

    // Build block proof.
    let pruned_block = MerkleProof::create(block_root.as_ref(), usage_tree)
        .prune_big_cells(true)
        .build_raw_ext(Cell::empty_context())?;

    if pruned_block.hash(0) != block_root.hash(0) {
        return Err(Error::InvalidData);
    }

    Ok(pruned_block)
}

pub fn make_key_block_proof<M>(block_root: Cell, with_prev_vset: bool) -> Result<Cell, Error>
where
    M: BlockchainModels,
{
    let usage_tree = UsageTree::new(UsageTreeMode::OnDataAccess);

    let tracked_root = usage_tree.track(&block_root);
    let raw_block = tracked_root.parse::<M::Block>()?;

    // Block info is required for key blocks to find the previous key block.
    // Only block info root cell is required (prev_ref is ignored).
    raw_block.load_info()?;

    // Access key block config.
    let extra = raw_block.load_extra()?;
    let custom = extra.load_custom()?.ok_or(Error::CellUnderflow)?;
    let config = custom.config().ok_or(Error::CellUnderflow)?;

    let current_vset = config.get_raw_cell_ref(34)?.ok_or(Error::CellUnderflow)?;
    current_vset.touch_recursive();

    if with_prev_vset {
        if let Some(prev_vset) = config.get_raw_cell_ref(32)? {
            prev_vset.touch_recursive();
        }
    }

    // Build block proof.
    let pruned_block = MerkleProof::create(block_root.as_ref(), usage_tree)
        .prune_big_cells(true)
        .build_raw_ext(Cell::empty_context())?;

    if pruned_block.hash(0) != block_root.hash(0) {
        return Err(Error::InvalidData);
    }

    Ok(pruned_block)
}

pub struct McProofForShard {
    pub root: Cell,
    pub latest_shard_seqno: u32,
}

/// Creates an mc block proof for the proof chain.
///
/// Input: pivot block.
pub fn make_mc_proof<M>(block_root: Cell, shard: ShardIdent) -> Result<McProofForShard, Error>
where
    M: BlockchainModels,
{
    let usage_tree = UsageTree::new(UsageTreeMode::OnDataAccess);

    let tracked_root = usage_tree.track(&block_root);
    let raw_block = tracked_root.parse::<M::Block>()?;

    // Block info is required for masterchain blocks to find the previous key block.
    // Only block info root cell is required (prev_ref is ignored).
    raw_block.load_info()?;

    // Access the required shard description.
    let extra = raw_block.load_extra()?;
    let custom = extra.load_custom()?.ok_or(Error::CellUnderflow)?;

    let latest_shard_seqno = custom.find_shard_seqno(shard)?;

    // Build block proof.
    let pruned_block = MerkleProof::create(block_root.as_ref(), usage_tree)
        .prune_big_cells(true)
        .build_raw_ext(Cell::empty_context())?;

    if pruned_block.hash(0) != block_root.hash(0) {
        return Err(Error::InvalidData);
    }

    Ok(McProofForShard {
        root: pruned_block,
        latest_shard_seqno,
    })
}

/// Creates a block with a single branch of the specified transaction.
///
/// Input: pruned block from [`make_pruned_block`].
pub fn make_tx_proof<M>(
    block_root: Cell,
    account: &HashBytes,
    lt: u64,
    include_info: bool,
) -> Result<Option<Cell>, Error>
where
    M: BlockchainModels,
{
    let usage_tree = UsageTree::new(UsageTreeMode::OnDataAccess);

    let tracked_root = usage_tree.track(&block_root);
    let raw_block = tracked_root.parse::<M::Block>()?;

    if include_info {
        let info = raw_block.load_info()?;
        // Touch `prev_ref` data to include it into the cell.
        info.prev_ref().data();
    }

    // Make a single branch with transaction.
    let extra = raw_block.load_extra()?;

    let account_blocks = extra.load_account_blocks()?;
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

    if pruned_block.hash(0) != block_root.hash(0) {
        return Err(Error::InvalidData);
    }

    Ok(Some(pruned_block))
}

fn find_shard_descr(mut root: &'_ DynCell, mut prefix: u64) -> Result<CellSlice<'_>, Error> {
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
    let mut cs = root.as_slice()?;
    if cs.load_bit()? {
        return Err(Error::InvalidTag);
    }

    Ok(cs)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::{Context, Result};
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

        // Pivot proof
        println!("building pivot proof");
        let pivot_proof = make_pivot_block_proof::<TychoModels>(false, block_root.clone())?;
        println!("SHARD PROOF: {}", Boc::encode_base64(pivot_proof));

        // Remove everything except transaction hashes.
        println!("building pruned block");
        let pruned_block = make_pruned_block::<TychoModels, _>(block_root, |_, _| Ok(()))?;

        // Build a pruned block which contains a single branch to transaction.
        println!("building tx proof");
        let tx_proof =
            make_tx_proof::<TychoModels>(Cell::virtualize(pruned_block), &account, lt, false)?
                .context("tx not found in block")?;

        // Done.
        println!("serializing tx proof");
        let pruned = Boc::encode_base64(tx_proof);

        println!("PRUNED: {pruned}");
        Ok(())
    }
}
