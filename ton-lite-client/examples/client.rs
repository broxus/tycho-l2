use std::net::SocketAddrV4;
use std::str::FromStr;

use anyhow::Result;
use base64::Engine;
use everscale_crypto::ed25519;
use ton_lite_client::client::LiteClient;
use ton_lite_client::config::LiteClientConfig;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let server_pubkey = ed25519::PublicKey::from_bytes(
        base64::engine::general_purpose::STANDARD
            .decode("n4VDnSCUuSpjnCyUk9e3QOOd6o0ItSWYbTnW3Wnn8wk=")?
            .try_into()
            .unwrap(),
    )
    .unwrap();

    let server_address = SocketAddrV4::from_str("5.9.10.47:19949")?;

    let config = LiteClientConfig::from_addr_and_keys(server_address, server_pubkey);
    let client = LiteClient::new(&config).await?;

    // Get last key block
    {
        // Get mc info
        let mc_block_id = client.get_last_mc_block_id().await?;

        // Get last mc block
        let mc_block = client.get_block(mc_block_id).await?;

        let prev_key_block_seqno = mc_block.block().load_info()?.prev_key_block_seqno;
        tracing::info!(prev_key_block_seqno);

        // Prev key block id
        let key_block_short_id = everscale_types::models::BlockIdShort {
            shard: mc_block_id.shard,
            seqno: prev_key_block_seqno,
        };
        let key_block_id = client.lookup_block(key_block_short_id).await?;

        // Prev key block
        let key_block = client.get_block(key_block_id.into()).await?;
        anyhow::ensure!(
            key_block.block().load_info()?.key_block,
            "invalid key block"
        );
    }

    Ok(())
}
