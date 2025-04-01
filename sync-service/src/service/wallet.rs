use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use everscale_types::abi::*;
use everscale_types::cell::Lazy;
use everscale_types::models::{
    AccountState, ExtInMsgInfo, Message, MsgInfo, OwnedRelaxedMessage, RelaxedIntMsgInfo,
    RelaxedMessage, RelaxedMsgInfo, StateInit, StdAddr, Transaction, ValidatorSet,
};
use everscale_types::num::{Tokens, Uint15};
use everscale_types::prelude::*;
use tycho_util::time::now_millis;

use crate::client::NetworkClient;
use crate::service::lib_store;
use crate::util::account::{compute_address, AccountStateResponse};

#[derive(Clone)]
#[repr(transparent)]
pub struct Wallet {
    inner: Arc<Inner>,
}

impl Wallet {
    pub fn new(
        workchain: i8,
        key: Arc<ed25519_dalek::SigningKey>,
        client: Arc<dyn NetworkClient>,
    ) -> Self {
        let address = compute_address(workchain, &make_state_init((*key).as_ref()));

        Self {
            inner: Arc::new(Inner {
                address,
                key,
                client,
            }),
        }
    }

    pub fn address(&self) -> &StdAddr {
        &self.inner.address
    }

    pub async fn deploy_vset_lib(
        &self,
        vset: &ValidatorSet,
        value: Tokens,
        id: u128,
    ) -> Result<StdAddr> {
        // Compute vset data and the lib_store address.
        let vset_data = lib_store::make_epoch_data(vset).context("failed to build epoch data")?;
        let state_init = lib_store::make_state_init(&self.inner.address, id);
        let address = compute_address(-1, &state_init);

        // Check that account doesn't exist.
        let client = self.inner.client.as_ref();
        let account = client
            .get_account_state(&address, None)
            .await
            .context("failed to get lib_store account")?;

        if let AccountStateResponse::Exists { account, .. } = account {
            match account.state {
                AccountState::Active { .. } | AccountState::Frozen { .. } => {
                    anyhow::bail!("lib_store account already exists: address={address}, id={id}");
                }
                AccountState::Uninit => {
                    tracing::warn!(
                        %address,
                        "lib_store account already exists, but uninit",
                    );
                }
            }
        }

        // Build internal message.
        let mut body = CellBuilder::new();
        body.store_reference(vset_data)?;
        let body = body.as_full_slice();

        let message = Lazy::new(&RelaxedMessage {
            info: RelaxedMsgInfo::Int(RelaxedIntMsgInfo {
                dst: address.clone().into(),
                ihr_disabled: true,
                value: value.into(),
                ..Default::default()
            }),
            init: Some(state_init),
            body,
            layout: None,
        })?;

        // Send message.
        let tx = self.send_message(0x1, message.cast_into(), 60).await?;
        tracing::info!(
            tx_hash = %tx.repr_hash(),
            %address,
            "sent lib_store deploy",
        );

        // Wait until lib_store contract is deployed.
        client.wait_for_deploy(&address).await;
        Ok(address)
    }

    pub async fn send_key_block(
        &self,
        key_block_proof: Cell,
        file_hash: &HashBytes,
        signatures: Cell,
        bridge_address: &StdAddr,
        value: Tokens,
        query_id: u64,
    ) -> Result<Lazy<Transaction>> {
        const METHOD_ID: u32 = 0x11a78ffe;

        // Build internal message.
        let mut body = CellBuilder::new();
        body.store_u32(METHOD_ID)?;
        body.store_reference(CellBuilder::build_from((file_hash, key_block_proof))?)?;
        body.store_reference(signatures)?;
        body.store_u64(query_id)?;
        let body = body.as_full_slice();

        let message = Lazy::new(&RelaxedMessage {
            info: RelaxedMsgInfo::Int(RelaxedIntMsgInfo {
                dst: bridge_address.clone().into(),
                ihr_disabled: true,
                bounce: true,
                value: value.into(),
                ..Default::default()
            }),
            init: None,
            body,
            layout: None,
        })?;

        let client = self.inner.client.as_ref();
        let bridge_account_state = client
            .get_account_state_with_retries(bridge_address, None)
            .await;
        let bridge_lt = match bridge_account_state {
            AccountStateResponse::Exists {
                last_transaction_id,
                ..
            } => last_transaction_id.lt,
            AccountStateResponse::NotExists { .. } => {
                anyhow::bail!("bridge account doesn't exist");
            }
            AccountStateResponse::Unchanged { .. } => {
                anyhow::bail!("unexpected response");
            }
        };

        // Send message.
        let tx = self.send_message(0x1, message.cast_into(), 60).await?;
        let out_msg = tx
            .load()?
            .out_msgs
            .get(Uint15::new(0))?
            .context("no outbound messages found")?;
        tracing::info!(
            tx_hash = %tx.repr_hash(),
            out_msg_hash = %out_msg.repr_hash(),
            "sent key block proof",
        );

        client
            .find_transaction(bridge_address, out_msg.repr_hash(), bridge_lt, None)
            .await
            .context("no tx found")
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

        let ttl = timeout.clamp(1, 60);

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
                    AccountState::Uninit => Some(make_state_init((*this.key).as_ref())),
                }
            }
            AccountStateResponse::Unchanged { .. } => anyhow::bail!("unexpected response"),
            AccountStateResponse::NotExists { .. } => anyhow::bail!("wallet does not exist"),
        };

        let now_ms = now_millis();
        let expire_at = (now_ms / 1000) as u32 + ttl;
        let body = methods::send_transaction()
            .encode_external(&inputs)
            .with_address(&this.address)
            .with_time(now_ms)
            .with_expire_at(expire_at)
            .with_pubkey((*this.key).as_ref())
            .build_input()?
            .sign(&this.key, signature_id)?;

        let message_cell = CellBuilder::build_from(Message {
            info: MsgInfo::ExtIn(ExtInMsgInfo {
                src: None,
                dst: this.address.clone().into(),
                ..Default::default()
            }),
            init,
            body: body.as_slice()?,
            layout: None,
        })?;

        this.client
            .send_message_reliable(&this.address, message_cell, known_lt, expire_at)
            .await
    }
}

struct Inner {
    address: StdAddr,
    key: Arc<ed25519_dalek::SigningKey>,
    client: Arc<dyn NetworkClient>,
}

pub fn make_state_init(pubkey: &ed25519_dalek::VerifyingKey) -> StateInit {
    StateInit {
        split_depth: None,
        special: None,
        code: Some(wallet_code().clone()),
        data: Some(CellBuilder::build_from((HashBytes::wrap(pubkey.as_bytes()), 0u64)).unwrap()),
        libraries: Dict::new(),
    }
}

pub fn wallet_code() -> &'static Cell {
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
        let pubkey = ed25519_dalek::VerifyingKey::from_bytes(&pubkey.0).unwrap();

        let state_init = make_state_init(&pubkey);
        let addr = compute_address(0, &state_init);
        assert_eq!(
            addr.to_string(),
            "0:2adb83beb873806e8971631173991e6250bd97310e8d09b5e2de44320d0a8068"
        );

        Ok(())
    }
}
