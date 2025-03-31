use anyhow::{Context, Result};
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{BlockId, BlockRef, ShardIdent, StdAddr, ValidatorSet};
use everscale_types::prelude::*;
use proof_api_util::block::{
    self, BlockchainBlock, BlockchainBlockExtra, BlockchainBlockMcExtra, BlockchainModels,
    TonModels,
};
use ton_lite_client::{proto, LiteClient};

#[derive(Clone)]
pub struct TonClient {
    lite_client: LiteClient,
}

impl TonClient {
    pub fn new(lite_client: LiteClient) -> Self {
        Self { lite_client }
    }

    // TODO: Move sync parts into rayon.
    pub async fn build_proof(
        &self,
        account: &StdAddr,
        lt: u64,
        tx_hash: &HashBytes,
    ) -> Result<Cell> {
        let block_id = self.find_transaction_block_id(account, lt, tx_hash).await?;
        tracing::debug!(%block_id, %tx_hash, "found transaction block id");

        let is_masterchain = account.is_masterchain();

        let block_root = self.lite_client.get_block(&block_id).await?;
        let tx_proof =
            block::make_tx_proof::<TonModels>(block_root, &account.address, lt, is_masterchain)
                .context("failed to build tx proof")?
                .context("tx not found in block")?;

        let mc_proof;
        let file_hash;
        let vset_utime_since;
        let signatures;
        let mut shard_proofs = Vec::new();
        if account.is_masterchain() {
            // No shard blocks are required in addition to masterchain proof.
            file_hash = block_id.file_hash;
            mc_proof = tx_proof;

            let prev_block_id = mc_proof
                .parse::<<TonModels as BlockchainModels>::Block>()?
                .load_info()?
                .prev_ref
                .parse::<BlockRef>()?
                .as_block_id(ShardIdent::MASTERCHAIN);

            // Find masterchain block proof.
            let mc_block_link = self
                .lite_client
                .get_block_proof(&prev_block_id, Some(&block_id), true)
                .await
                .context("failed to get mc block proof")?;

            // Build signatures dict.
            let mc = parse_mc_block_proof(mc_block_link, &block_id)?;
            vset_utime_since = mc.vset_utime_since;
            signatures = mc.signatures;
        } else {
            // Find masterchain block id and get all proof links until the shard block.
            let proto::ShardBlockProof { mc_block_id, links } = self
                .lite_client
                .get_shard_block_proof(&block_id)
                .await
                .context("failed to get shard block proof")?;

            file_hash = mc_block_id.file_hash;

            // Find previous masterchain block id.
            let prev_block_id = self
                .lite_client
                .lookup_block(mc_block_id.as_short_id().saturating_prev())
                .await
                .context("failed to get prev block id")?;

            // Find masterchain block proof.
            let mc_block_link = self
                .lite_client
                .get_block_proof(&prev_block_id, Some(&mc_block_id), true)
                .await
                .context("failed to get mc block proof")?;

            // Build signatures dict.
            let mc = parse_mc_block_proof(mc_block_link, &mc_block_id)?;
            vset_utime_since = mc.vset_utime_since;
            signatures = mc.signatures;

            let mut expected_hash = mc_block_id.root_hash;

            let mut mc_extra_root = None;
            for link in links {
                let block_root = Boc::decode(link.proof)
                    .context("failed to deserialize shard block proof")?
                    .parse_exotic::<MerkleProof>()
                    .context("failed to load shard block proof")?
                    .cell;

                anyhow::ensure!(
                    *block_root.hash(0) == expected_hash,
                    "proof link hash mismatch"
                );

                expected_hash = link.block_id.root_hash;
                if mc_extra_root.is_none() {
                    mc_extra_root = Some(block_root);
                    continue;
                }

                let proof = block::make_pivot_block_proof::<TonModels>(false, block_root)
                    .context("failed to build pivot block proof")?;
                shard_proofs.push(proof);
            }

            shard_proofs.push(tx_proof);

            mc_proof = merge_mc_block_proof(
                mc.header_proof,
                mc_extra_root.context("masterchain extra root not found")?,
                block_id.shard,
            )?;
        }

        let proof_chain = block::make_proof_chain(
            &file_hash,
            mc_proof,
            &shard_proofs,
            vset_utime_since,
            signatures,
        )?;
        Ok(proof_chain)
    }

    async fn find_transaction_block_id(
        &self,
        account: &StdAddr,
        lt: u64,
        tx_hash: &HashBytes,
    ) -> Result<BlockId> {
        let list = self
            .lite_client
            .get_transactions(account, lt, tx_hash, 1)
            .await
            .context("failed to find transaction")?;

        let mut block_ids = list.block_ids.into_iter();
        let Some(block_id) = block_ids.next() else {
            anyhow::bail!("liteserver returned no block ids");
        };
        anyhow::ensure!(
            block_ids.next().is_none(),
            "liteserver returned unexpected block ids"
        );

        Ok(block_id)
    }
}

fn parse_current_vset<T: AsRef<[u8]>>(config_proof: T) -> Result<ValidatorSet> {
    let proof = Boc::decode(config_proof)?.parse_exotic::<MerkleProof>()?;
    let block = proof
        .cell
        .parse::<<TonModels as BlockchainModels>::Block>()?;

    block
        .load_extra()?
        .load_custom()?
        .context("config proof without custom")?
        .config()
        .context("expected key block")?
        .get_current_validator_set()
        .context("failed to load current vset")
}

fn parse_mc_block_proof(
    partial: proto::PartialBlockProof,
    mc_block_id: &BlockId,
) -> Result<McProof> {
    let forward = 'proof: {
        for step in partial.steps {
            if let proto::BlockLink::BlockLinkForward(step) = step {
                break 'proof step;
            }
        }

        anyhow::bail!("forward proof step not found");
    };

    anyhow::ensure!(forward.to == *mc_block_id, "proof link id mismatch");

    let vset = parse_current_vset(forward.config_proof).context("failed to config proof")?;
    let signatures =
        block::prepare_signatures(forward.signatures.signatures.into_iter().map(Ok), &vset)
            .context("failed to prepare block signature")?;

    Ok(McProof {
        header_proof: forward.dest_proof,
        vset_utime_since: vset.utime_since,
        signatures,
    })
}

struct McProof {
    header_proof: Vec<u8>,
    vset_utime_since: u32,
    signatures: Cell,
}

fn merge_mc_block_proof(
    header_proof: Vec<u8>,
    extra_proof: Cell,
    shard_ident: ShardIdent,
) -> Result<Cell> {
    // Parse proof for block info.
    let header_proof = Boc::decode(header_proof)
        .context("failed to parse mc header proof")?
        .parse_exotic::<MerkleProof>()?
        .cell;

    // Make sure that this is the expected block.
    anyhow::ensure!(
        header_proof.hash(0) == extra_proof.hash(0),
        "header proof id mismatch"
    );

    // Combine two blocks into one with both info and extra.
    let mut extra_cs = extra_proof.as_slice()?;
    let pruned_info = extra_cs.load_reference()?;

    let info = header_proof.as_slice()?.load_reference_cloned()?;
    anyhow::ensure!(
        pruned_info.hash(0) == info.repr_hash(),
        "info hash mismatch"
    );

    let proof =
        CellBuilder::build_from((info, extra_cs)).context("failed to build mc block proof")?;

    // Minimize the proof.
    let proof = block::make_mc_proof::<TonModels>(proof.clone(), shard_ident)?.root;

    // Done.
    Ok(proof)
}
