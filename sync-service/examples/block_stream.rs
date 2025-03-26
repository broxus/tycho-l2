use std::net::SocketAddrV4;
use std::str::FromStr;

use anyhow::Result;
use everscale_types::cell::HashBytes;
use sync_service::stream::ton;
use ton_lite_client::{LiteClient, LiteClientConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let server_pubkey = "n4VDnSCUuSpjnCyUk9e3QOOd6o0ItSWYbTnW3Wnn8wk=".parse::<HashBytes>()?;
    let server_address = SocketAddrV4::from_str("5.9.10.47:19949")?;

    let config = LiteClientConfig::from_addr_and_keys(server_address, server_pubkey);
    let client = LiteClient::new(&config).await?;

    let stream = ton::BlockStream::new(client);
    while let Some(block) = stream.next_block().await {
        tracing::info!(block = block.seqno);
    }

    Ok(())
}
