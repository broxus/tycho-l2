use crate::stream::KeyBlockInfo;

pub struct BlockStream {}

impl BlockStream {
    pub async fn next_block(&self) -> Option<KeyBlockInfo> {
        todo!()
    }
}
