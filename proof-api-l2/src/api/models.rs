use std::str::FromStr;

use everscale_types::models::{StdAddr, StdAddrFormat};
use schemars::schema::Schema;
use schemars::{JsonSchema, SchemaGenerator};
use serde::{Deserialize, Serialize};

/// General error response.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "error")]
pub enum ErrorResponse {
    Internal { message: String },
    NotFound { message: &'static str },
}

/// API version and build information.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiInfoResponse {
    pub version: String,
    pub build: String,
}

/// Block proof chain for an existing transaction.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProofChainResponse {
    /// Base64 encoded BOC with the proof chain.
    pub proof_chain: String,
}

// TODO: Move into util.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonAddr(#[serde(with = "serde_ton_address")] pub StdAddr);

// TODO: Move into util.
impl FromStr for TonAddr {
    type Err = everscale_types::error::ParseAddrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, _) = StdAddr::from_str_ext(s, StdAddrFormat::any())?;
        Ok(Self(addr))
    }
}

// TODO: Move into util.
impl JsonSchema for TonAddr {
    fn schema_name() -> String {
        "Address".to_string()
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let schema = gen.subschema_for::<String>();
        let mut schema = schema.into_object();
        schema.metadata().description = Some("StdAddr in any format".to_string());
        schema.format = Some("0:[0-9a-fA-F]{64}".to_string());
        schema.metadata().examples = vec![serde_json::json!(
            "0:3333333333333333333333333333333333333333333333333333333333333333"
        )];
        schema.into()
    }
}

// TODO: Move into util.
pub mod serde_ton_address {
    use everscale_types::models::{StdAddr, StdAddrBase64Repr};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<StdAddr, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StdAddrBase64Repr::<true>::deserialize(deserializer)
    }

    pub fn serialize<S>(addr: &StdAddr, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        StdAddrBase64Repr::<true>::serialize(addr, serializer)
    }
}
