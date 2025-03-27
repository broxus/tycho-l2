use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use proof_api_ton::client::TonClient;
use ton_lite_client::{LiteClient, LiteClientConfig, TonGlobalConfig};

#[derive(Parser)]
pub struct Cmd {
    // Path to the TON global config.
    #[clap(long)]
    pub global_config: PathBuf,
}

impl Cmd {
    pub async fn run(self) -> Result<()> {
        let global_config = TonGlobalConfig::load_from_file(self.global_config)?;
        let lite_client = LiteClient::new(LiteClientConfig::default(), global_config.liteservers);
        let client = TonClient::new(lite_client);

        Ok(())
    }
}
