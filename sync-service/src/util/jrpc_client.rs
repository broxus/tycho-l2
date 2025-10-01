use std::marker::PhantomData;

use anyhow::{Context, Result};
use reqwest::{IntoUrl, Url};
use serde::{Deserialize, Serialize};
use tycho_types::models::{BlockId, BlockchainConfig, StdAddr};
use tycho_types::prelude::*;
use tycho_util::serde_helpers;

use crate::util::account::AccountStateResponse;

pub struct JrpcClient {
    client: reqwest::Client,
    base_url: Url,
}

impl JrpcClient {
    pub fn new<U: IntoUrl>(base_url: U) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        let client = reqwest::ClientBuilder::new()
            .default_headers(headers)
            .build()
            .context("failed to build http client")?;

        Ok(Self {
            client,
            base_url: base_url.into_url()?,
        })
    }

    pub async fn send_message(&self, message: &DynCell) -> Result<()> {
        #[derive(Serialize)]
        struct Params<'a> {
            #[serde(with = "Boc")]
            message: &'a DynCell,
        }

        self.post(&JrpcRequest {
            method: "sendMessage",
            params: &Params { message },
        })
        .await
        .context("failed to send message")
    }

    pub async fn get_library_cell(&self, hash: &HashBytes) -> Result<LibraryCellResponse> {
        #[derive(Serialize)]
        struct Params<'a> {
            hash: &'a HashBytes,
        }

        self.post(&JrpcRequest {
            method: "getLibraryCell",
            params: &Params { hash },
        })
        .await
        .context("failed to get library cell")
    }

    pub async fn get_transactions(
        &self,
        account: &StdAddr,
        last_transaction_lt: Option<u64>,
        limit: u8,
    ) -> Result<Vec<String>> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            account: &'a StdAddr,
            #[serde(with = "serde_helpers::string")]
            last_transaction_lt: u64,
            limit: u8,
        }

        self.post(&JrpcRequest {
            method: "getTransactionsList",
            params: &Params {
                account,
                last_transaction_lt: last_transaction_lt.unwrap_or(u64::MAX),
                limit,
            },
        })
        .await
        .context("failed to get transactions list")
    }

    pub async fn get_dst_transaction(&self, message_hash: &HashBytes) -> Result<Option<String>> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            message_hash: &'a HashBytes,
        }

        self.post(&JrpcRequest {
            method: "getDstTransaction",
            params: &Params { message_hash },
        })
        .await
        .context("failed to get dst transaction")
    }

    pub async fn get_latest_config(&self) -> Result<LatestBlockchainConfigResponse> {
        self.post(&JrpcRequest {
            method: "getBlockchainConfig",
            params: &(),
        })
        .await
        .context("failed to get blockchain config")
    }

    pub async fn get_key_block_proof(&self, seqno: u32) -> Result<BlockProofResponse> {
        #[derive(Debug, Serialize)]
        struct Params {
            seqno: u32,
        }

        self.post(&JrpcRequest {
            method: "getKeyBlockProof",
            params: &Params { seqno },
        })
        .await
        .context("failed to get key block proof")
    }

    pub async fn get_account_state(
        &self,
        address: &StdAddr,
        last_transaction_lt: Option<u64>,
    ) -> Result<AccountStateResponse> {
        #[derive(Serialize)]
        struct Params<'a> {
            address: &'a StdAddr,
            #[serde(default, with = "serde_helpers::option_string")]
            last_transaction_lt: Option<u64>,
        }

        self.post(&JrpcRequest {
            method: "getContractState",
            params: &Params {
                address,
                last_transaction_lt,
            },
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
        tracing::trace!(res);

        match serde_json::from_str(&res).context("invalid JRPC response")? {
            JrpcResponse::Success(res) => Ok(res),
            JrpcResponse::Err(err) => anyhow::bail!(err),
        }
    }
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
#[serde(rename_all = "camelCase")]
pub struct BlockProofResponse {
    #[serde(default, with = "serde_helpers::option_string")]
    pub block_id: Option<BlockId>,
    pub proof: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LibraryCellResponse {
    #[serde(with = "Boc")]
    pub cell: Option<Cell>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn jrpc_works() -> Result<()> {
        let client = JrpcClient::new("https://rpc-testnet.tychoprotocol.com/rpc")?;

        let latest_config = client.get_latest_config().await?;
        println!("Seqno: {}", latest_config.seqno);

        let addr = "-1:3333333333333333333333333333333333333333333333333333333333333333".parse()?;
        let txs = client.get_transactions(&addr, None, 10).await?;
        assert_eq!(txs.len(), 10);

        let msg = "0000000000000000000000000000000000000000000000000000000000000000".parse()?;
        let tx = client.get_dst_transaction(&msg).await?;
        assert!(tx.is_none());

        let msg = "ed2b462c6a1bd3b4b4f67f5c812d6b271eff1e3553389e0d5430489cc393bfa7".parse()?;
        let tx = client.get_dst_transaction(&msg).await?;
        assert!(tx.is_some());

        let state = client.get_account_state(&addr, None).await?;
        matches!(state, AccountStateResponse::Exists { .. });

        Ok(())
    }
}
