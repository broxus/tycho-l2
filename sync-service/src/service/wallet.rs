use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use everscale_crypto::ed25519;
use everscale_types::abi::*;
use everscale_types::cell::Lazy;
use everscale_types::models::{
    AccountState, ExtInMsgInfo, MsgInfo, OwnedMessage, OwnedRelaxedMessage, StateInit, StdAddr,
    Transaction,
};
use everscale_types::prelude::*;
use tycho_util::time::now_millis;

use crate::client::NetworkClient;
use crate::util::account::{AccountStateResponse, LastTransactionId};

#[derive(Clone)]
#[repr(transparent)]
pub struct Wallet {
    inner: Arc<Inner>,
}

impl Wallet {
    pub fn new(
        workchain: i8,
        keypair: Arc<ed25519::KeyPair>,
        client: Arc<dyn NetworkClient>,
    ) -> Self {
        let state_init = make_state_init(&keypair.public_key);
        let address = StdAddr::new(
            workchain,
            *CellBuilder::build_from(state_init).unwrap().repr_hash(),
        );

        Self {
            inner: Arc::new(Inner {
                address,
                keypair,
                client,
            }),
        }
    }

    pub fn address(&self) -> &StdAddr {
        &self.inner.address
    }

    pub async fn send_message(
        &self,
        flags: u8,
        message: Lazy<OwnedRelaxedMessage>,
        timeout: u32,
    ) -> Result<Lazy<Transaction>> {
        let this = self.inner.as_ref();

        let signature_id = this
            .client
            .get_signature_id()
            .await
            .context("failed to get signature id")?;

        let ttl = timeout.clamp(1, 60) as u32;

        let AbiValue::Tuple(inputs) = methods::SendTransactionInputs {
            flags,
            message: message.into_inner(),
        }
        .into_abi() else {
            unreachable!();
        };

        // TODO: Wait for balance.
        let known_lt;
        let init = match this.client.get_account_state(&this.address, None).await? {
            AccountStateResponse::Exists {
                account,
                last_transaction_id,
                ..
            } => {
                known_lt = last_transaction_id.lt;
                match &account.state {
                    AccountState::Active(..) => None,
                    AccountState::Frozen(..) => anyhow::bail!("wallet is frozen"),
                    AccountState::Uninit => Some(make_state_init(&this.keypair.public_key)),
                }
            }
            AccountStateResponse::Unchanged { .. } => anyhow::bail!("unexpected response"),
            AccountStateResponse::NotExists { .. } => anyhow::bail!("wallet does not exist"),
        };

        let pubkey =
            ed25519_dalek::VerifyingKey::from_bytes(self.inner.keypair.public_key.as_bytes())
                .unwrap();

        let now_ms = now_millis();
        let expire_at = (now_ms / 1000) as u32 + ttl;
        let unsigned_body = methods::send_transaction()
            .encode_external(&inputs)
            .with_address(&self.inner.address)
            .with_time(now_ms)
            .with_expire_at(expire_at)
            .with_pubkey(&pubkey)
            .build_input()?;

        let body = {
            let to_sign = extend_signature_with_id(unsigned_body.hash.as_slice(), signature_id);
            let signature = this.keypair.sign_raw(&to_sign);
            unsigned_body.fill_signature(Some(&signature))?
        };

        let body_range = CellSliceRange::full(body.as_ref());

        let message = OwnedMessage {
            info: MsgInfo::ExtIn(ExtInMsgInfo {
                src: None,
                dst: this.address.clone().into(),
                ..Default::default()
            }),
            init,
            body: (body, body_range),
            layout: None,
        };
        let message_cell = CellBuilder::build_from(message)?;

        this.send_external(known_lt, message_cell, expire_at).await
    }
}

struct Inner {
    address: StdAddr,
    keypair: Arc<ed25519::KeyPair>,
    client: Arc<dyn NetworkClient>,
}

impl Inner {
    async fn send_external(
        &self,
        mut known_lt: u64,
        msg: Cell,
        expire_at: u32,
    ) -> Result<Lazy<Transaction>> {
        const POLL_INTERVAL: Duration = Duration::from_secs(1);
        const RETRY_INTERVAL: Duration = Duration::from_secs(1);
        const BATCH_LEN: u8 = 10;

        let msg_hash = *msg.repr_hash();

        self.client
            .send_message(msg)
            .await
            .context("failed to send message")?;

        let account = &self.address;
        let client = self.client.as_ref();
        let get_state = async |known_lt: u64| loop {
            match client.get_account_state(account, Some(known_lt)).await {
                Ok(res) => break res,
                Err(e) => {
                    tracing::warn!(
                        client = client.name(),
                        "failed to get contract state: {e:?}"
                    );
                    tokio::time::sleep(RETRY_INTERVAL).await;
                }
            }
        };

        let do_find_transaction = async |mut last: LastTransactionId, known_lt: u64| loop {
            tracing::debug!(%account, ?last, known_lt, "fetching transactions");
            let res = client
                .get_transactions(account, last.lt, &last.hash, BATCH_LEN)
                .await?;
            anyhow::ensure!(!res.is_empty(), "got empty transactions response");

            for raw_tx in res {
                let hash = raw_tx.repr_hash();
                anyhow::ensure!(*hash == last.hash, "last tx hash mismatch");
                let tx = raw_tx.load().context("got invalid transaction")?;
                anyhow::ensure!(tx.lt == last.lt, "last tx lt mismatch");

                if let Some(in_msg) = &tx.in_msg {
                    if *in_msg.repr_hash() == msg_hash {
                        return Ok(Some(raw_tx));
                    }
                }

                last = LastTransactionId {
                    lt: tx.prev_trans_lt,
                    hash: tx.prev_trans_hash,
                };
                if tx.prev_trans_lt <= known_lt {
                    break;
                }
            }

            if last.lt <= known_lt {
                return Ok(None);
            }
        };
        let find_transaction = async |last: LastTransactionId, known_lt: u64| loop {
            match do_find_transaction(last, known_lt).await {
                Ok(res) => break res,
                Err(e) => {
                    tracing::warn!(
                        client = client.name(),
                        "failed to process transactions: {e:?}",
                    );
                    tokio::time::sleep(RETRY_INTERVAL).await;
                }
            }
        };

        loop {
            let timings = match get_state(known_lt).await {
                AccountStateResponse::Exists {
                    timings,
                    last_transaction_id,
                    ..
                } => {
                    if last_transaction_id.lt > known_lt {
                        if let Some(tx) = find_transaction(last_transaction_id, known_lt).await {
                            return Ok(tx);
                        }

                        known_lt = last_transaction_id.lt;
                        tracing::debug!(%account, known_lt, "got new known lt");
                    }

                    timings
                }
                AccountStateResponse::NotExists { timings }
                | AccountStateResponse::Unchanged { timings } => timings,
            };

            // Message expired.
            if timings.gen_utime > expire_at {
                anyhow::bail!("message expired");
            }

            tracing::trace!(known_lt, %msg_hash, expire_at, "poll account");
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

fn make_state_init(pubkey: &ed25519::PublicKey) -> StateInit {
    StateInit {
        split_depth: None,
        special: None,
        code: Some(wallet_code().clone()),
        data: Some(CellBuilder::build_from((HashBytes::wrap(pubkey.as_bytes()), 0u64)).unwrap()),
        libraries: Dict::new(),
    }
}

fn wallet_code() -> &'static Cell {
    static CODE: OnceLock<Cell> = OnceLock::new();
    CODE.get_or_init(|| Boc::decode(include_bytes!("../../res/wallet_code.boc")).unwrap())
}

mod methods {
    use super::*;

    pub fn send_transaction() -> &'static Function {
        static FUNCTION: OnceLock<Function> = OnceLock::new();
        FUNCTION.get_or_init(move || {
            Function::builder(AbiVersion::V2_3, "sendTransactionRaw")
                .with_id(0x169e3e11)
                .with_headers([
                    AbiHeaderType::PublicKey,
                    AbiHeaderType::Time,
                    AbiHeaderType::Expire,
                ])
                .with_inputs(SendTransactionInputs::abi_type().named("").flatten())
                .build()
        })
    }

    #[derive(Debug, Clone)]
    pub struct SendTransactionInputs {
        pub flags: u8,
        pub message: Cell,
    }

    // TODO: Replace with macros
    impl WithAbiType for SendTransactionInputs {
        fn abi_type() -> AbiType {
            AbiType::tuple([
                u8::abi_type().named("flags"),
                Cell::abi_type().named("message"),
            ])
        }
    }

    // TODO: Replace with macros
    impl IntoAbi for SendTransactionInputs {
        fn as_abi(&self) -> AbiValue {
            AbiValue::tuple([
                self.flags.as_abi().named("flags"),
                self.message.as_abi().named("message"),
            ])
        }

        fn into_abi(self) -> AbiValue
        where
            Self: Sized,
        {
            self.as_abi()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_address() -> Result<()> {
        let pubkey = "b5621766abc6482b9ba0c986215c218d2a9c462c597dc89e5ecae889a1063adb"
            .parse::<HashBytes>()?;
        let pubkey = ed25519::PublicKey::from_bytes(pubkey.0).unwrap();

        let state_init = make_state_init(&pubkey);

        let addr = StdAddr::new(0, *CellBuilder::build_from(state_init).unwrap().repr_hash());
        assert_eq!(
            addr.to_string(),
            "0:2adb83beb873806e8971631173991e6250bd97310e8d09b5e2de44320d0a8068"
        );

        Ok(())
    }
}
