use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use proof_api_ton::api::ApiConfig;
use proof_api_ton::client::TonClient;
use proof_api_util::api::Api;
use serde::{Deserialize, Serialize};
use ton_lite_client::{LiteClient, LiteClientConfig, TonGlobalConfig};
use tycho_util::cli::logger::LoggerConfig;

#[derive(Parser)]
pub struct Cmd {
    /// dump the template of the config
    #[clap(
        short = 'i',
        long,
        conflicts_with_all = ["config", "global_config", "logger_config"]
    )]
    pub init_config: Option<PathBuf>,

    /// overwrite the existing config
    #[clap(short, long)]
    pub force: bool,

    // Path to the TON global config.
    #[clap(long)]
    pub global_config: PathBuf,

    /// path to the node config
    #[clap(long, required_unless_present = "init_config")]
    pub config: Option<PathBuf>,

    /// path to the logger config
    #[clap(long)]
    pub logger_config: Option<PathBuf>,
}

impl Cmd {
    pub async fn run(self) -> Result<()> {
        std::panic::set_hook(Box::new(|info| {
            use std::io::Write;
            let backtrace = std::backtrace::Backtrace::capture();

            tracing::error!("{info}\n{backtrace}");
            std::io::stderr().flush().ok();
            std::io::stdout().flush().ok();
            std::process::exit(1);
        }));

        if let Some(config_path) = self.init_config {
            if config_path.exists() && !self.force {
                anyhow::bail!("config file already exists, use --force to overwrite");
            }

            let config = Config::default();
            std::fs::write(config_path, serde_json::to_string_pretty(&config).unwrap())?;
            return Ok(());
        }

        let config = Config::load_from_file(self.config.as_ref().context("no config")?)?;
        tycho_util::cli::logger::init_logger(&config.logger_config, self.logger_config)?;

        let global_config = TonGlobalConfig::load_from_file(self.global_config)?;
        let lite_client = LiteClient::new(LiteClientConfig::default(), global_config.liteservers);
        let client = TonClient::new(lite_client);

        let api = Api::bind(
            config.api.listen_addr,
            proof_api_ton::api::build_api(&config.api, client)
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .context("failed to bind API service")?;
        tracing::info!("created api");

        api.serve().await.map_err(Into::into)
    }
}

#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    api: ApiConfig,
    logger_config: LoggerConfig,
}

impl Config {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = std::fs::read(path).context("failed to read config")?;
        serde_json::from_slice(&data).context("failed to deserialize config")
    }
}
