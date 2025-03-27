use std::str::FromStr;

use anyhow::Result;
use everscale_types::models::StdAddr;
use nekoton_abi::execution_context::ExecutionContextBuilder;
use sync_service::utils::jrpc_client::{AccountStateResponse, JrpcClient};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let url = reqwest::Url::parse("https://rpc-devnet1.tychoprotocol.com/")?;
    let client = JrpcClient::new(url)?;

    {
        let addr = StdAddr::from_str(
            "0:457c0ac35986d4e056deee8428abe27294f97c3266dc9062d689a07c8e967164",
        )?;
        let account = match client.get_account(&addr).await? {
            AccountStateResponse::Exists { account, .. } => account,
            _ => unreachable!(),
        };

        let config = client.get_config().await?;

        let context = ExecutionContextBuilder::new(&account)
            .with_config(config.config)
            .build()?;

        let result = context.run_getter("get_state_short", &[])?;
        assert!(result.success);

        let stack = result.stack;

        let current_epoch_since: u32 = stack[0].try_as_int()?.try_into()?;
        tracing::info!(current_epoch_since);
    }

    Ok(())
}
