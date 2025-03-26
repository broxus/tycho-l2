use anyhow::Result;
use everscale_types::models::StdAddr;
use everscale_types::prelude::*;
use ton_lite_client::LiteClient;

pub struct TonClient {
    lite_client: LiteClient,
}

impl TonClient {
    pub async fn build_proof(&self, account: &StdAddr, lt: u64) -> Result<Option<Cell>> {
        // Done
        Ok(())
    }
}
