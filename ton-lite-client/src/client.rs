use std::time::Duration;

use anyhow::Result;
use everscale_crypto::ed25519;
use tl_proto::{TlRead, TlWrite};

use crate::config::LiteClientConfig;
use crate::proto;
use crate::tcp_adnl::{TcpAdnl, TcpAdnlConfig, TcpAdnlError};
use crate::utils::FromResponse;

#[derive(Clone)]
pub struct LiteClient {
    tcp_adnl: TcpAdnl,
    query_timeout: Duration,
}

impl LiteClient {
    pub async fn new(config: &LiteClientConfig) -> Result<Self> {
        // Generate client keys
        let rng = &mut rand::thread_rng();
        let client_secret = ed25519::SecretKey::generate(rng);

        let tcp_adnl = TcpAdnl::connect(TcpAdnlConfig {
            client_secret,
            server_address: config.server_address.into(),
            server_pubkey: config.server_pubkey,
            connection_timeout: config.connection_timeout,
        })
        .await
        .map_err(LiteClientError::ConnectionFailed)?;

        let query_timeout = config.query_timeout;

        Ok(Self {
            tcp_adnl,
            query_timeout,
        })
    }

    pub async fn get_version(&self) -> Result<proto::Version> {
        self.query(proto::GetVersion).await
    }

    pub async fn get_masterchain_info(&self) -> Result<proto::MasterchainInfo> {
        self.query(proto::GetMasterchainInfo).await
    }

    pub async fn send_message<T: AsRef<[u8]>>(&self, message: T) -> Result<proto::SendMsgStatus> {
        self.query(proto::SendMessage {
            body: message.as_ref(),
        })
        .await
    }

    async fn query<Q, R>(&self, query: Q) -> Result<R>
    where
        Q: TlWrite<Repr = tl_proto::Boxed>,
        R: FromResponse,
        for<'a> R: TlRead<'a>,
    {
        match self.tcp_adnl.query(query, self.query_timeout).await {
            Ok(Some(res)) => <R>::from_response(res),
            Ok(None) => Err(LiteClientError::QueryTimeout.into()),
            Err(e) => Err(LiteClientError::QueryFailed(e).into()),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum LiteClientError {
    #[error("connection failed")]
    ConnectionFailed(#[source] TcpAdnlError),
    #[error("query failed")]
    QueryFailed(#[source] TcpAdnlError),
    #[error("query timeout")]
    QueryTimeout,
}
