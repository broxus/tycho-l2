use std::str::FromStr;

use everscale_types::models::{StdAddr, StdAddrFormat};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonAddr(#[serde(with = "ton_address")] pub StdAddr);

impl FromStr for TonAddr {
    type Err = everscale_types::error::ParseAddrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, _) = StdAddr::from_str_ext(s, StdAddrFormat::any())?;
        Ok(Self(addr))
    }
}

#[cfg(feature = "api")]
impl schemars::JsonSchema for TonAddr {
    fn schema_name() -> String {
        "Address".to_string()
    }

    fn json_schema(gen: &mut schemars::SchemaGenerator) -> schemars::schema::Schema {
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

pub mod ton_address {
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
