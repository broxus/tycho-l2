use std::time::Duration;

use anyhow::Result;
use everscale_crypto::ed25519;
use everscale_types::cell::HashBytes;
use everscale_types::merkle::MerkleProof;
use everscale_types::models::{BlockId, BlockIdShort, BlockSignature, Signature};
use tl_proto::{TlRead, TlWrite};

use crate::config::LiteClientConfig;
use crate::models::block::{BlockProof, BlockShort, BlockStuff};
use crate::proto;
use crate::proto::BlockLink;
use crate::tcp_adnl::{TcpAdnl, TcpAdnlConfig, TcpAdnlError};

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

    pub async fn get_version(&self) -> Result<u32> {
        let version = self.query::<_, proto::Version>(proto::GetVersion).await?;
        Ok(version.version)
    }

    pub async fn get_last_mc_block_id(&self) -> Result<BlockId> {
        let info = self
            .query::<_, proto::MasterchainInfo>(proto::GetMasterchainInfo)
            .await?;
        Ok(info.last.into())
    }

    pub async fn send_message<T: AsRef<[u8]>>(&self, message: T) -> Result<u32> {
        let status = self
            .query::<_, proto::SendMsgStatus>(proto::SendMessage {
                body: message.as_ref(),
            })
            .await?;

        Ok(status.status)
    }

    pub async fn get_block(&self, id: BlockId) -> Result<BlockStuff> {
        let block = self
            .query::<_, proto::BlockData>(proto::GetBlock { id: id.into() })
            .await?;
        BlockStuff::new(&block.data, block.id.into())
    }

    pub async fn lookup_block(&self, id: BlockIdShort) -> Result<BlockId> {
        let block_header = self
            .query::<_, proto::BlockHeader>(proto::LookupBlock {
                mode: (),
                id: id.into(),
                seqno: Some(()),
                lt: None,
                utime: None,
            })
            .await?;

        Ok(block_header.id.into())
    }

    pub async fn get_block_proof(&self, from: BlockId, to: Option<BlockId>) -> Result<BlockProof> {
        let block_proof = self
            .query::<_, proto::PartialBlockProof>(proto::GetBlockProof {
                mode: (),
                known_block: from.into(),
                target_block: to.map(Into::into),
            })
            .await?;

        for block_link in block_proof.steps {
            match block_link {
                BlockLink::BlockLinkBack { .. } => {}
                BlockLink::BlockLinkForward {
                    config_proof,
                    signatures,
                    ..
                } => {
                    let proof = everscale_types::boc::Boc::decode(&config_proof)?
                        .parse_exotic::<MerkleProof>()?;

                    let block = proof.cell.virtualize().parse::<BlockShort>()?;
                    let extra = block.load_extra()?;

                    let config = extra.load_config()?;

                    let signatures = signatures
                        .signatures
                        .into_iter()
                        .map(|x| -> Result<BlockSignature> {
                            Ok(BlockSignature {
                                node_id_short: HashBytes::from(x.node_id_short),
                                signature: Signature(
                                    x.signature
                                        .try_into()
                                        .map_err(|_e| LiteClientError::InvalidBlockProof)?,
                                ),
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;

                    return Ok(BlockProof {
                        signatures,
                        validator_set: config.get_current_validator_set()?,
                    });
                }
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
