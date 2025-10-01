use serde::{Deserialize, Serialize};
use tycho_types::models::{Account, StateInit, StdAddr};
use tycho_types::prelude::*;
use tycho_util::serde_helpers;

pub fn compute_address(workchain: i8, state_init: &StateInit) -> StdAddr {
    StdAddr::new(
        workchain,
        *CellBuilder::build_from(state_init).unwrap().repr_hash(),
    )
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

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct LastTransactionId {
    #[serde(with = "serde_helpers::string")]
    pub lt: u64,
    pub hash: HashBytes,
}
