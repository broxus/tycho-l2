use std::net::SocketAddrV4;
use std::time::Duration;

use base64::Engine;
use broxus_util::{const_duration_ms, serde_duration_ms};
use everscale_crypto::ed25519;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteClientConfig {
    /// Server socket address
    pub server_address: SocketAddrV4,

    /// Server pubkey
    #[serde(with = "serde_public_key")]
    pub server_pubkey: ed25519::PublicKey,

    /// Server connection timeout
    #[serde(with = "serde_duration_ms", default = "const_duration_ms::<2000>")]
    pub connection_timeout: Duration,

    /// Server query timeout
    #[serde(with = "serde_duration_ms", default = "const_duration_ms::<10000>")]
    pub query_timeout: Duration,
}

impl LiteClientConfig {
    pub fn from_addr_and_keys(addr: SocketAddrV4, server_key: ed25519::PublicKey) -> Self {
        Self {
            server_address: addr,
            server_pubkey: server_key,
            connection_timeout: Duration::from_millis(5000),
            query_timeout: Duration::from_millis(10000),
        }
    }
}

mod serde_public_key {
    use serde::{Deserializer, Serializer};

    use super::*;

    pub fn serialize<S: Serializer>(
        public: &ed25519::PublicKey,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(public.as_bytes()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<ed25519::PublicKey, D::Error> {
        use serde::de::Error;

        let str = String::deserialize(deserializer)?;
        let bytes = match hex::decode(&str) {
            Ok(bytes) if bytes.len() == 32 => bytes,
            _ => match base64::engine::general_purpose::STANDARD.decode(&str) {
                Ok(bytes) => bytes,
                Err(_) => return Err(Error::custom("invalid pubkey string")),
            },
        };

        let bytes = bytes
            .try_into()
            .map_err(|_e| Error::custom("invalid pubkey length"))?;

        ed25519::PublicKey::from_bytes(bytes).ok_or_else(|| Error::custom("invalid pubkey"))
    }
}
