use std::marker::PhantomData;

use anyhow::{Context, Result};
use everscale_types::models::{Account, BlockchainConfig, StdAddr};
use everscale_types::prelude::*;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tycho_util::serde_helpers;

pub struct JrpcClient {
    client: reqwest::Client,
    base_url: Url,
}

impl JrpcClient {
    pub fn new(base_url: Url) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        let client = reqwest::ClientBuilder::new()
            .default_headers(headers)
            .build()
            .context("failed to build http client")?;

        Ok(Self { client, base_url })
    }

    pub async fn send_message(&self, message: &DynCell) -> Result<()> {
        #[derive(Serialize)]
        struct Params<'a> {
            // TODO: Revert this line once the fix is merged:
            // #[serde(with = "Boc")]
            message: &'a DynCell,
        }

        self.post(&JrpcRequest {
            method: "sendMessage",
            params: &Params { message },
        })
        .await
        .context("failed to send message")
    }

    pub async fn get_config(&self) -> Result<LatestBlockchainConfigResponse> {
        self.post(&JrpcRequest {
            method: "getBlockchainConfig",
            params: &(),
        })
        .await
        .context("failed to get blockchain config")
    }

    pub async fn get_latest_key_block(&self) -> Result<LatestKeyBlockResponse> {
        self.post(&JrpcRequest {
            method: "getLatestKeyBlock",
            params: &(),
        })
        .await
        .context("failed to get latest key block")
    }

    pub async fn get_key_block_proof(&self, seqno: u32) -> Result<BlockProofResponse> {
        self.post(&JrpcRequest {
            method: "getKeyBlockProof",
            params: &KeyBlockProofRequest { seqno },
        })
        .await
    }

    pub async fn get_account(&self, address: &StdAddr) -> Result<AccountStateResponse> {
        #[derive(Serialize)]
        struct Params<'a> {
            address: &'a StdAddr,
        }

        self.post(&JrpcRequest {
            method: "getContractState",
            params: &Params { address },
        })
        .await
        .context("failed to get account state")
    }

    pub async fn post<Q, R>(&self, data: &Q) -> Result<R>
    where
        Q: Serialize,
        for<'de> R: Deserialize<'de>,
    {
        let response = self
            .client
            .post(self.base_url.clone())
            .json(data)
            .send()
            .await?;

        let res = response.text().await?;
        tracing::debug!(res);

        match serde_json::from_str(&res).context("invalid JRPC response")? {
            JrpcResponse::Success(res) => Ok(res),
            JrpcResponse::Err(err) => anyhow::bail!(err),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct KeyBlockProofRequest {
    pub seqno: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestBlockchainConfigResponse {
    pub global_id: i32,
    pub seqno: u32,
    #[serde(with = "BocRepr")]
    pub config: BlockchainConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LatestKeyBlockResponse {
    pub block: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockProofResponse {
    pub proof: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
#[allow(unused)]
pub enum AccountStateResponse {
    NotExists {
        timings: GenTimings,
    },
    #[serde(rename_all = "camelCase")]
    Exists {
        #[serde(deserialize_with = "deserialize_account")]
        account: Box<Account>,
        timings: GenTimings,
        last_transaction_id: LastTransactionId,
    },
    Unchanged {
        timings: GenTimings,
    },
}

fn deserialize_account<'de, D>(deserializer: D) -> Result<Box<Account>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use everscale_types::cell::Load;
    use serde::de::Error;

    fn read_account(cell: Cell) -> Result<Box<Account>, everscale_types::error::Error> {
        let s = &mut cell.as_slice()?;
        Ok(Box::new(Account {
            address: <_>::load_from(s)?,
            storage_stat: <_>::load_from(s)?,
            last_trans_lt: <_>::load_from(s)?,
            balance: <_>::load_from(s)?,
            state: <_>::load_from(s)?,
        }))
    }

    Boc::deserialize(deserializer).and_then(|cell| read_account(cell).map_err(Error::custom))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenTimings {
    #[serde(with = "serde_helpers::string")]
    pub gen_lt: u64,
    pub gen_utime: u32,
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct LastTransactionId {
    #[serde(with = "serde_helpers::string")]
    pub lt: u64,
    pub hash: HashBytes,
}

struct JrpcRequest<'a, T> {
    method: &'a str,
    params: &'a T,
}

impl<T: Serialize> Serialize for JrpcRequest<'_, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut ser = serializer.serialize_struct("JrpcRequest", 4)?;
        ser.serialize_field("jsonrpc", "2.0")?;
        ser.serialize_field("id", &1)?;
        ser.serialize_field("method", self.method)?;
        ser.serialize_field("params", self.params)?;
        ser.end()
    }
}

enum JrpcResponse<T> {
    Success(T),
    Err(Box<serde_json::value::RawValue>),
}

impl<'de, T> Deserialize<'de> for JrpcResponse<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(de: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "lowercase")]
        enum Field {
            Result,
            Error,
            #[serde(other)]
            Other,
        }

        enum ResponseData<T> {
            Result(T),
            Error(Box<serde_json::value::RawValue>),
        }

        struct ResponseVisitor<T>(PhantomData<T>);

        impl<'de, T> serde::de::Visitor<'de> for ResponseVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = ResponseData<T>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON-RPC response object")
            }

            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut result = None::<ResponseData<T>>;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Result if result.is_none() => {
                            result = Some(map.next_value().map(ResponseData::Result)?);
                        }
                        Field::Error if result.is_none() => {
                            result = Some(map.next_value().map(ResponseData::Error)?);
                        }
                        Field::Other => {
                            map.next_value::<&serde_json::value::RawValue>()?;
                        }
                        Field::Result => return Err(serde::de::Error::duplicate_field("result")),
                        Field::Error => return Err(serde::de::Error::duplicate_field("error")),
                    }
                }

                result.ok_or_else(|| serde::de::Error::missing_field("result or error"))
            }
        }

        Ok(match de.deserialize_map(ResponseVisitor(PhantomData))? {
            ResponseData::Result(result) => JrpcResponse::Success(result),
            ResponseData::Error(error) => JrpcResponse::Err(error),
        })
    }
}
