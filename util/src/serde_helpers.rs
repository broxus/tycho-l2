#[cfg(feature = "api")]
use std::borrow::Cow;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use tycho_types::models::{StdAddr, StdAddrFormat};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonAddr(#[serde(with = "ton_address")] pub StdAddr);

impl FromStr for TonAddr {
    type Err = tycho_types::error::ParseAddrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, _) = StdAddr::from_str_ext(s, StdAddrFormat::any())?;
        Ok(Self(addr))
    }
}

#[cfg(feature = "api")]
impl schemars::JsonSchema for TonAddr {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Address")
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut schema = generator.subschema_for::<String>();
        let object = schema.ensure_object();
        object.insert("description".into(), "StdAddr in any format".into());
        object.insert("format".into(), "0:[0-9a-fA-F]{64}".into());
        object.insert(
            "examples".into(),
            vec![serde_json::json!(
                "0:3333333333333333333333333333333333333333333333333333333333333333"
            )]
            .into(),
        );
        schema
    }
}

pub mod ton_address {
    use tycho_types::models::{StdAddr, StdAddrBase64Repr};

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
