use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tycho_crypto::ed25519;
use tycho_util::serde_helpers;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LiteClientConfig {
    /// Server connection timeout.
    #[serde(with = "serde_helpers::humantime")]
    pub connection_timeout: Duration,

    /// Server query timeout.
    #[serde(with = "serde_helpers::humantime")]
    pub query_timeout: Duration,

    // Interval before connection attempts.
    #[serde(with = "serde_helpers::humantime")]
    pub reconnect_interval: Duration,
}

impl Default for LiteClientConfig {
    fn default() -> Self {
        Self {
            connection_timeout: Duration::from_secs(5),
            query_timeout: Duration::from_secs(10),
            reconnect_interval: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeInfo {
    pub address: SocketAddr,
    #[serde(with = "serde_pubkey")]
    pub pubkey: ed25519::PublicKey,
}

#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TonGlobalConfig {
    #[serde(with = "serde_liteservers")]
    pub liteservers: Vec<NodeInfo>,
}

impl TonGlobalConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = std::fs::read(path).context("failed to read global config")?;
        serde_json::from_slice(&data).context("failed to deserialize global config")
    }
}

mod serde_pubkey {
    use std::str::FromStr;

    use serde::de::Error;
    use serde::{Deserialize, Deserializer};
    use tycho_crypto::ed25519;
    use tycho_types::cell::HashBytes;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ed25519::PublicKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let pubkey = String::deserialize(deserializer)?;
        let pubkey = HashBytes::from_str(&pubkey).map_err(Error::custom)?;
        ed25519::PublicKey::from_bytes(pubkey.0).ok_or_else(|| Error::custom("invalid pubkey"))
    }
}

mod serde_liteservers {
    use std::net::Ipv4Addr;

    use serde::{Deserialize, Deserializer};

    use super::*;

    #[derive(Deserialize)]
    struct NodeId {
        #[serde(with = "serde_pubkey")]
        key: ed25519::PublicKey,
    }

    #[derive(Deserialize)]
    struct Node {
        ip: i32,
        port: u16,
        id: NodeId,
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<NodeInfo>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Vec::<Node>::deserialize(deserializer)?
            .into_iter()
            .map(|item| NodeInfo {
                address: SocketAddr::from((Ipv4Addr::from_bits(item.ip as u32), item.port)),
                pubkey: item.id.key,
            })
            .collect())
    }
}
