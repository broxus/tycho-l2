use std::time::Duration;

use anyhow::Result;
use everscale_types::models::{BlockId, BlockIdShort};
use everscale_types::prelude::*;
use tl_proto::{TlRead, TlWrite};

use crate::config::LiteClientConfig;
use crate::proto;
use crate::tcp_adnl::{TcpAdnl, TcpAdnlConfig, TcpAdnlError};

#[derive(Clone)]
pub struct LiteClient {
    tcp_adnl: TcpAdnl,
    query_timeout: Duration,
}

impl LiteClient {
    pub async fn new(config: &LiteClientConfig) -> Result<Self> {
        let tcp_adnl = TcpAdnl::connect(TcpAdnlConfig {
            server_address: config.server_address,
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

    pub async fn get_version(&self) -> Result<u32> {
        let version = self
            .query::<_, proto::Version>(proto::rpc::GetVersion)
            .await?;
        Ok(version.version)
    }

    pub async fn get_last_mc_block_id(&self) -> Result<BlockId> {
        let info = self
            .query::<_, proto::MasterchainInfo>(proto::rpc::GetMasterchainInfo)
            .await?;
        Ok(info.last)
    }

    pub async fn send_message<T: AsRef<[u8]>>(&self, message: T) -> Result<u32> {
        let status = self
            .query::<_, proto::SendMsgStatus>(proto::rpc::SendMessage {
                body: message.as_ref(),
            })
            .await?;

        Ok(status.status)
    }

    pub async fn get_block(&self, id: &BlockId) -> Result<Cell> {
        let block = self
            .query::<_, proto::BlockData>(proto::rpc::GetBlock { id: *id })
            .await?;

        let cell = Boc::decode(block.data)?;
        anyhow::ensure!(*cell.repr_hash() == id.root_hash, "root hash mismatch");
        Ok(cell)
    }

    pub async fn lookup_block(&self, id: BlockIdShort) -> Result<BlockId> {
        let block_header = self
            .query::<_, proto::BlockHeader>(proto::rpc::LookupBlock {
                mode: (),
                id,
                seqno: Some(()),
                lt: None,
                utime: None,
            })
            .await?;

        Ok(block_header.id)
    }

    pub async fn get_block_proof(&self, block_id: &BlockId) -> Result<proto::BlockLinkForward> {
        let block_proof = self
            .query::<_, proto::PartialBlockProof>(proto::rpc::GetBlockProof {
                mode: (),
                known_block: *block_id,
                target_block: None,
            })
            .await?;

        for block_link in block_proof.steps {
            match block_link {
                proto::BlockLink::BlockLinkBack { .. } => break,
                proto::BlockLink::BlockLinkForward(proof) => return Ok(proof),
            }
        }

        Err(LiteClientError::InvalidBlockProof.into())
    }

    async fn query<Q, R>(&self, query: Q) -> Result<R>
    where
        Q: TlWrite<Repr = tl_proto::Boxed>,
        for<'a> R: TlRead<'a>,
    {
        enum QueryResponse<T> {
            Ok(T),
            Err(String),
        }

        impl<'a, R> tl_proto::TlRead<'a> for QueryResponse<R>
        where
            R: TlRead<'a>,
        {
            type Repr = tl_proto::Boxed;

            fn read_from(packet: &mut &'a [u8]) -> tl_proto::TlResult<Self> {
                let constructor = { <u32 as TlRead>::read_from(&mut packet.as_ref())? };
                if constructor == proto::Error::TL_ID {
                    let proto::Error { message, .. } = <_>::read_from(packet)?;
                    Ok(QueryResponse::Err(message))
                } else {
                    <R>::read_from(packet).map(QueryResponse::Ok)
                }
            }
        }

        match self.tcp_adnl.query(query, self.query_timeout).await {
            Ok(Some(QueryResponse::Ok(data))) => Ok(data),
            Ok(Some(QueryResponse::Err(message))) => Err(anyhow::Error::msg(message)),
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
    #[error("invalid block proof")]
    InvalidBlockProof,
}
