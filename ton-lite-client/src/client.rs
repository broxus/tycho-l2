use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use arc_swap::{ArcSwapAny, ArcSwapOption};
use everscale_crypto::ed25519;
use everscale_types::models::{BlockId, BlockIdShort, StdAddr};
use everscale_types::prelude::*;
use tl_proto::{TlRead, TlWrite};
use tokio::sync::Notify;
use tycho_util::futures::JoinTask;

use crate::config::{LiteClientConfig, NodeInfo};
use crate::proto;
use crate::tcp_adnl::{TcpAdnl, TcpAdnlError};

#[derive(Clone)]
pub struct LiteClient {
    inner: Arc<Inner>,
}

impl LiteClient {
    pub fn new<I>(config: LiteClientConfig, nodes: I) -> Self
    where
        I: IntoIterator<Item = NodeInfo>,
    {
        let state = Arc::new(ActiveState {
            any_connected: Notify::new(),
            active_count: AtomicUsize::new(0),
            connections: nodes
                .into_iter()
                .map(|node| ConnectionState {
                    address: node.address,
                    pubkey: node.pubkey,
                    client: ArcSwapAny::new(None),
                })
                .collect(),
            connection_timeout: config.connection_timeout,
            reconnect_interval: config.reconnect_interval,
        });

        let handles = spawn_connections(&state);

        Self {
            inner: Arc::new(Inner {
                query_timeout: config.query_timeout,
                state,
                counter: AtomicUsize::new(0),
                _handles: handles,
            }),
        }
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

    pub async fn get_block_proof(
        &self,
        block_id: &BlockId,
        target_block: Option<&BlockId>,
        with_known_block: bool,
    ) -> Result<proto::PartialBlockProof> {
        self.query::<_, proto::PartialBlockProof>(proto::rpc::GetBlockProof {
            mode: (),
            known_block: *block_id,
            target_block: target_block.copied(),
            with_known_block: with_known_block.then_some(()),
        })
        .await
    }

    pub async fn get_shard_block_proof(
        &self,
        block_id: &BlockId,
    ) -> Result<proto::ShardBlockProof> {
        self.query::<_, proto::ShardBlockProof>(proto::rpc::GetShardBlockProof {
            block_id: *block_id,
        })
        .await
    }

    pub async fn get_config(&self, block_id: &BlockId) -> Result<proto::ConfigInfo> {
        self.query::<_, proto::ConfigInfo>(proto::rpc::GetConfigAll {
            mode: (),
            id: *block_id,
        })
        .await
    }

    pub async fn get_transactions(
        &self,
        account: &StdAddr,
        lt: u64,
        hash: &HashBytes,
        count: u32,
    ) -> Result<proto::TransactionList> {
        self.query::<_, proto::TransactionList>(proto::rpc::GetTransactions {
            account: account.clone(),
            lt,
            count,
            hash: hash.0,
        })
        .await
    }

    pub async fn get_account(
        &self,
        block_id: BlockId,
        account: StdAddr,
    ) -> Result<proto::AccountState> {
        self.query::<_, proto::AccountState>(proto::rpc::GetAccountState {
            id: block_id,
            account,
        })
        .await
    }

    pub async fn query<Q, R>(&self, query: Q) -> Result<R>
    where
        Q: TlWrite<Repr = tl_proto::Boxed>,
        for<'a> R: TlRead<'a>,
    {
        enum QueryResponse<T> {
            Ok(T),
            Err(Box<str>),
        }

        impl<'a, R> tl_proto::TlRead<'a> for QueryResponse<R>
        where
            R: TlRead<'a>,
        {
            type Repr = tl_proto::Boxed;

            #[allow(clippy::useless_asref)]
            fn read_from(packet: &mut &'a [u8]) -> tl_proto::TlResult<Self> {
                let constructor = { <u32 as TlRead>::read_from(&mut packet.as_ref())? }; // Don't touch!!!
                if constructor == proto::Error::TL_ID {
                    let proto::Error { message, .. } = <_>::read_from(packet)?;
                    Ok(QueryResponse::Err(message))
                } else {
                    <R>::read_from(packet).map(QueryResponse::Ok)
                }
            }
        }

        const MAX_ATTEMPTS: usize = 200;
        const MAX_ERRORS: usize = 20;

        let query = tl_proto::serialize(query);
        let query = tl_proto::RawBytes::<tl_proto::Boxed>::new(&query);

        let state = self.inner.state.as_ref();
        let connection_count = state.connections.len();
        if connection_count == 0 {
            return Err(LiteClientError::NoConnections.into());
        }

        let mut id = self.inner.counter.fetch_add(1, Ordering::Relaxed) % connection_count;

        let mut attempts = 0usize;
        let mut error_count = 0usize;
        loop {
            let connection = &state.connections[id];
            tracing::debug!(id, attempts, error_count, addr = %connection.address, "trying to send query");

            let e = match connection.client.load_full() {
                // Client is ready.
                Some(client) => {
                    let fut = client.query(query);

                    match tokio::time::timeout(self.inner.query_timeout, fut).await {
                        Ok(Ok(QueryResponse::Ok(data))) => break Ok(data),
                        Ok(Ok(QueryResponse::Err(e))) => LiteClientError::ErrorResponse(e),
                        Ok(Err(e)) => LiteClientError::QueryFailed(e),
                        Err(_) => LiteClientError::Timeout,
                    }
                }
                // Client is still connecting.
                None => {
                    id = (id + 1) % connection_count;
                    attempts += 1;

                    if attempts > MAX_ATTEMPTS {
                        return Err(LiteClientError::NoConnections.into());
                    }

                    if attempts >= connection_count {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }

                    continue;
                }
            };

            tracing::debug!(id, attempts, error_count, addr = %connection.address, "query failed: {e:?}");

            id = (id + 1) % connection_count;
            if matches!(&e, LiteClientError::Timeout) {
                continue;
            }

            error_count += 1;
            if error_count > MAX_ERRORS {
                return Err(e.into());
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

struct Inner {
    query_timeout: Duration,
    state: Arc<ActiveState>,
    counter: AtomicUsize,
    _handles: Vec<JoinTask<()>>,
}

struct ConnectionState {
    address: SocketAddr,
    pubkey: ed25519::PublicKey,
    client: ArcSwapOption<TcpAdnl>,
}

struct ActiveState {
    any_connected: Notify,
    active_count: AtomicUsize,
    connections: Vec<ConnectionState>,
    connection_timeout: Duration,
    reconnect_interval: Duration,
}

fn spawn_connections(state: &Arc<ActiveState>) -> Vec<JoinTask<()>> {
    let mut tasks = Vec::new();

    for i in 0..state.connections.len() {
        let state = state.clone();
        tasks.push(JoinTask::new(async move {
            let connection = &state.connections[i];

            loop {
                'connection: {
                    tracing::debug!(addr = ?connection.address, "connecting to lite client");

                    let fut = TcpAdnl::connect(connection.address, connection.pubkey);
                    let client = match tokio::time::timeout(state.connection_timeout, fut).await {
                        Ok(res) => match res {
                            Ok(client) => Arc::new(client),
                            Err(e) => {
                                tracing::debug!(
                                    addr = ?connection.address,
                                    "connection failed: {e:?}",
                                );
                                break 'connection;
                            }
                        },
                        Err(_) => {
                            tracing::debug!(addr = ?connection.address, "connection timeout");
                            break 'connection;
                        }
                    };

                    connection.client.store(Some(client.clone()));

                    state.active_count.fetch_add(1, Ordering::Release);
                    state.any_connected.notify_waiters();
                    client.wait_closed().await;
                    state.active_count.fetch_sub(1, Ordering::Release);

                    connection.client.store(None);

                    tracing::debug!(addr = ?connection.address, "connection closed");
                }

                tokio::time::sleep(state.reconnect_interval).await;
            }
        }));
    }

    tasks
}

#[derive(thiserror::Error, Debug)]
pub enum LiteClientError {
    #[error("no connections available")]
    NoConnections,
    #[error("query failed: {0}")]
    ErrorResponse(Box<str>),
    #[error("query failed")]
    QueryFailed(#[source] TcpAdnlError),
    #[error("timeout")]
    Timeout,
}
