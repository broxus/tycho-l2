use std::net::SocketAddr;
use std::time::Duration;

use everscale_types::cell::HashBytes;
use serde::{Deserialize, Serialize};
use tycho_util::serde_helpers;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteClientConfig {
    /// Server socket address
    pub server_address: SocketAddr,

    /// Server pubkey
    pub server_pubkey: HashBytes,

    /// Server connection timeout.
    #[serde(
        with = "serde_helpers::humantime",
        default = "const_duration_ms::<5000>"
    )]
    pub connection_timeout: Duration,

    /// Server query timeout.
    #[serde(
        with = "serde_helpers::humantime",
        default = "const_duration_ms::<10000>"
    )]
    pub query_timeout: Duration,
}

impl LiteClientConfig {
    pub fn from_addr_and_keys<S>(server_address: S, server_pubkey: HashBytes) -> Self
    where
        S: Into<SocketAddr>,
    {
        Self {
            server_address: server_address.into(),
            server_pubkey,
            connection_timeout: Duration::from_millis(5000),
            query_timeout: Duration::from_millis(10000),
        }
    }
}

const fn const_duration_ms<const MS: u64>() -> Duration {
    Duration::from_millis(MS)
}
