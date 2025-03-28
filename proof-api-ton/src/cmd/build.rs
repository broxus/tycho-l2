use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use everscale_types::boc::Boc;
use everscale_types::cell::HashBytes;
use everscale_types::error::ParseAddrError;
use everscale_types::models::{StdAddr, StdAddrFormat};
use proof_api_ton::client::TonClient;
use ton_lite_client::{LiteClient, LiteClientConfig, TonGlobalConfig};

/// Build transaction proof.
#[derive(Parser)]
pub struct Cmd {
    /// Account address.
    #[clap(value_parser = parse_addr)]
    address: StdAddr,

    /// Transaction logical time.
    lt: u64,

    /// Transaction hash.
    hash: HashBytes,

    // Path to the TON global config.
    #[clap(long)]
    global_config: PathBuf,
}

impl Cmd {
    #[allow(clippy::print_stdout)]
    pub async fn run(self) -> Result<()> {
        tracing_subscriber::fmt::init();

        let global_config = TonGlobalConfig::load_from_file(self.global_config)?;
        let lite_client = LiteClient::new(LiteClientConfig::default(), global_config.liteservers);
        let client = TonClient::new(lite_client);

        let proof_chain = client
            .build_proof(&self.address, self.lt, &self.hash)
            .await
            .context("failed to build proof")?;

        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "proof_chain": Boc::encode_base64(proof_chain),
            }))?,
        );
        Ok(())
    }
}

fn parse_addr(addr: &str) -> Result<StdAddr, ParseAddrError> {
    let (addr, _) = StdAddr::from_str_ext(addr, StdAddrFormat::any())?;
    Ok(addr)
}
