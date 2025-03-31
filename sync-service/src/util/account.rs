use everscale_types::models::Account;
use everscale_types::prelude::*;
use serde::{Deserialize, Serialize};
use tycho_util::serde_helpers;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
#[allow(unused)]
pub enum AccountStateResponse {
    NotExists {
        timings: GenTimings,
    },
    #[serde(rename_all = "camelCase")]
    Exists {
        #[serde(with = "BocRepr")]
        account: Box<Account>,
        timings: GenTimings,
        last_transaction_id: LastTransactionId,
    },
    Unchanged {
        timings: GenTimings,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenTimings {
    #[serde(with = "serde_helpers::string")]
    pub gen_lt: u64,
    pub gen_utime: u32,
}

#[derive(Debug, Deserialize)]
pub struct LastTransactionId {
    #[serde(with = "serde_helpers::string")]
    pub lt: u64,
    pub hash: HashBytes,
}
