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

    // Server info
    let server_pubkey = ed25519::PublicKey::from_bytes(
        base64::engine::general_purpose::STANDARD
            .decode("n4VDnSCUuSpjnCyUk9e3QOOd6o0ItSWYbTnW3Wnn8wk=")?
            .try_into()
            .unwrap(),
    )
    .unwrap();

    let server_address = SocketAddrV4::from_str("5.9.10.47:19949")?;

    // Lite Client
    let config = LiteClientConfig::from_addr_and_keys(server_address, server_pubkey);

    let client = LiteClient::new(&config).await?;

    let version = client.get_version().await?;
    tracing::info!(?version);

    Ok(())
}
