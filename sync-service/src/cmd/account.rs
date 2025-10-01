use anyhow::Result;
use clap::Parser;
use rand::Rng;
use serde::Serialize;
use sync_service::service::wallet;
use sync_service::util::account::compute_address;
use tycho_types::cell::HashBytes;
use tycho_types::models::StdAddr;

/// Generate new account.
#[derive(Parser)]
pub struct Cmd {
    /// Workchain.
    #[clap(short, long, default_value_t = 0)]
    pub workchain: i8,

    /// Optional account public key.
    #[clap(long, value_parser = parse_pubkey)]
    pub public: Option<ed25519_dalek::VerifyingKey>,
}

impl Cmd {
    #[allow(clippy::print_stdout)]
    pub async fn run(self) -> Result<()> {
        let mut secret = None;
        let public = match self.public {
            Some(public) => public,
            None => {
                let secret = secret.insert(rand::rng().random::<ed25519_dalek::SecretKey>());
                *ed25519_dalek::SigningKey::from_bytes(secret).as_ref()
            }
        };

        let state_init = wallet::make_state_init(&public);
        let address = compute_address(self.workchain, &state_init);

        #[derive(Serialize)]
        struct Output<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            secret: Option<HashBytes>,
            public: &'a HashBytes,
            address: StdAddr,
        }

        println!(
            "{}",
            serde_json::to_string_pretty(&Output {
                secret: secret.map(HashBytes),
                public: HashBytes::wrap(public.as_bytes()),
                address,
            })?,
        );
        Ok(())
    }
}

fn parse_pubkey(s: &str) -> Result<ed25519_dalek::VerifyingKey> {
    let pubkey = s.parse::<HashBytes>()?;
    ed25519_dalek::VerifyingKey::from_bytes(pubkey.as_array()).map_err(Into::into)
}
