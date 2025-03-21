use anyhow::Result;
use everscale_types::models::BlockId;

use crate::models::block::Block;

pub struct BlockStuff {
    id: BlockId,
    block: Block,
}

impl BlockStuff {
    pub fn new(data: &[u8], id: BlockId) -> Result<Self> {
        let file_hash = everscale_types::boc::Boc::file_hash(data);
        anyhow::ensure!(id.file_hash == file_hash.0, "wrong file_hash");

        let root = everscale_types::boc::Boc::decode(data)?;
        anyhow::ensure!(id.root_hash == root.repr_hash().0, "wrong root hash");

        let block = root.parse::<Block>()?;
        Ok(Self { id, block })
    }

    #[inline(always)]
    pub fn id(&self) -> &BlockId {
        &self.id
    }

    #[inline(always)]
    pub fn block(&self) -> &Block {
        &self.block
    }
}
