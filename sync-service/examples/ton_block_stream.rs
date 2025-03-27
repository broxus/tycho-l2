use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;

use anyhow::Result;
use everscale_types::cell::HashBytes;
use sync_service::stream::BlockStream;
use ton_lite_client::{LiteClient, LiteClientConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let server_pubkey = "aF91CuUHuuOv9rm2W5+O/4h38M3sRm40DtSdRxQhmtQ=".parse::<HashBytes>()?;
    let server_address = SocketAddrV4::new(Ipv4Addr::from_bits(-2018135749i32 as u32), 53312);

    let config = LiteClientConfig::from_addr_and_keys(server_address, server_pubkey);
    let client = LiteClient::new(&config).await?;

    let stream = BlockStream::new(client);
    while let Some(block) = stream.next_block().await {
        tracing::info!(prev_seqno = block.prev_seqno);
    }

    Ok(())
}
