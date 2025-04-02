use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use ctr::cipher::{KeyIvInit, StreamCipher};
use everscale_crypto::ed25519;
use rand::{Rng, RngCore};
use sha2::Digest;
use tl_proto::{IntermediateBytes, TlRead, TlWrite};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify};
use tycho_util::futures::JoinTask;
use tycho_util::sync::CancellationFlag;

use self::queries_cache::QueriesCache;
use crate::proto;

mod queries_cache;

pub struct TcpAdnl {
    state: SharedState,
}

impl TcpAdnl {
    pub async fn connect<S>(
        address: S,
        server_pubkey: ed25519::PublicKey,
    ) -> Result<Self, TcpAdnlError>
    where
        S: tokio::net::ToSocketAddrs,
    {
        let (socket_rx, mut socket_tx) = TcpStream::connect(address)
            .await
            .map_err(TcpAdnlError::ConnectionError)?
            .into_split();

        let mut initial_buffer = vec![0; 160];
        rand::thread_rng().fill_bytes(&mut initial_buffer);

        let cipher_receive = Aes256Ctr::new(
            generic_array::GenericArray::from_slice(&initial_buffer[0..32]),
            generic_array::GenericArray::from_slice(&initial_buffer[64..80]),
        );
        let cipher_send = Aes256Ctr::new(
            generic_array::GenericArray::from_slice(&initial_buffer[32..64]),
            generic_array::GenericArray::from_slice(&initial_buffer[80..96]),
        );

        let client_secret = rand::thread_rng().gen::<ed25519::SecretKey>();

        build_handshake_packet(&server_pubkey, &client_secret, &mut initial_buffer);

        let queries_cache = Arc::new(QueriesCache::default());
        let closed = Closed::default();
        let receiver = JoinTask::new(socket_reader(
            socket_rx,
            cipher_receive,
            queries_cache.clone(),
            closed.clone(),
        ));

        socket_tx
            .write_all(&initial_buffer)
            .await
            .map_err(TcpAdnlError::ConnectionError)?;

        let state = SharedState {
            queries_cache,
            query_id: Default::default(),
            sender: Arc::new(Mutex::new(Sender {
                cipher: cipher_send,
                socket: socket_tx,
            })),
            closed,
            _receiver: receiver,
        };

        Ok(Self { state })
    }

    pub fn is_closed(&self) -> bool {
        self.state.closed.is_closed()
    }

    pub fn wait_closed(&self) -> impl Future<Output = ()> + Send + Sync + 'static {
        let closed = self.state.closed.clone();
        async move {
            // NOTE: Aquire `Notified` future before the flag check.
            let notified = closed.inner.notify.notified();

            if !closed.is_closed() {
                notified.await;
            }
        }
    }

    pub async fn query<Q, R>(&self, query: Q) -> Result<R, TcpAdnlError>
    where
        Q: TlWrite<Repr = tl_proto::Boxed>,
        for<'a> R: TlRead<'a>,
    {
        let seqno = self.state.query_id.fetch_add(1, Ordering::Relaxed);
        let mut query_id = [0; 32];
        query_id[..std::mem::size_of::<usize>()].copy_from_slice(&seqno.to_le_bytes());

        let query = proto::LiteQuery {
            wrapped_request: IntermediateBytes(proto::WrappedQuery {
                wait_masterchain_seqno: None,
                query,
            }),
        };

        let mut data = tl_proto::serialize(proto::AdnlMessageQuery {
            query_id: &query_id,
            query: IntermediateBytes(query),
        });

        let pending_query = self.state.queries_cache.add_query(query_id);

        let cancelled = CancellationFlag::new();
        scopeguard::defer! {
            cancelled.cancel();
        }

        let cancelled = cancelled.clone();
        let sender = self.state.sender.clone();
        let handle = tokio::task::spawn(async move {
            if cancelled.check() {
                return Ok(());
            }

            let mut sender = sender.lock().await;
            if cancelled.check() {
                return Ok(());
            }

            encrypt_data(&mut sender.cipher, &mut data);
            sender.socket.write_all(&data).await
        });

        let query = pin!(pending_query.wait());
        let res = match futures_util::future::select(handle, query).await {
            futures_util::future::Either::Left((sent, right)) => {
                sent.map_err(|_e| TcpAdnlError::SocketClosed)?
                    .map_err(TcpAdnlError::ConnectionError)?;
                right.await
            }
            futures_util::future::Either::Right((left, _)) => left,
        };

        match res {
            Some(res) => tl_proto::deserialize(&res).map_err(TcpAdnlError::InvalidAnswer),
            None => Err(TcpAdnlError::DuplicateQuery),
        }
    }
}

#[derive(Default, Clone)]
struct Closed {
    inner: Arc<ClosedInner>,
}

impl Closed {
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
    }
}

#[derive(Default)]
struct ClosedInner {
    closed: AtomicBool,
    notify: Notify,
}

struct SharedState {
    queries_cache: Arc<QueriesCache>,
    query_id: AtomicUsize,
    sender: Arc<Mutex<Sender>>,
    closed: Closed,
    _receiver: JoinTask<std::io::Error>,
}

struct Sender {
    cipher: Aes256Ctr,
    socket: OwnedWriteHalf,
}

async fn socket_reader<T>(
    mut socket: T,
    mut cipher: Aes256Ctr,
    queries_cache: Arc<QueriesCache>,
    closed: Closed,
) -> std::io::Error
where
    T: AsyncRead + Unpin,
{
    const MIN_PACKET_LEN: usize = 4 + 32 + 32;

    scopeguard::defer! {
        closed.close();
        tracing::debug!("socket reader finished");
    }

    let mut packet = Vec::with_capacity(4096);

    let mut buffer = [0u8; 4096];
    let mut target_len = None::<usize>;
    'outer: loop {
        match socket.read(&mut buffer).await {
            Ok(0) => {
                return std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "socket closed")
            }
            Ok(n) => {
                let buffer = &mut buffer[..n];
                cipher.apply_keystream(buffer);
                packet.extend_from_slice(buffer);
            }
            Err(e) => {
                tracing::debug!(?e, "socket error");
                return e;
            }
        }

        while packet.len() >= target_len.unwrap_or(MIN_PACKET_LEN) {
            let length = u32::from_le_bytes(packet[..4].try_into().unwrap()) as usize + 4;
            if packet.len() < length {
                target_len = Some(length);
                continue 'outer;
            }

            'packet: {
                if length < MIN_PACKET_LEN {
                    tracing::warn!("too small packet");
                    break 'packet;
                }

                if !sha2::Sha256::digest(&packet[4..length - 32])
                    .as_slice()
                    .eq(&packet[length - 32..length])
                {
                    tracing::warn!("packet checksum mismatch");
                    break 'packet;
                }

                let data = &packet[4 + 32..length - 32];
                if data.is_empty() {
                    break 'packet;
                }

                match tl_proto::deserialize::<proto::AdnlMessageAnswer<'_>>(data) {
                    Ok(proto::AdnlMessageAnswer { query_id, data }) => {
                        queries_cache.update_query(query_id, data);
                    }
                    Err(e) => tracing::warn!("invalid response: {e:?}"),
                };
            };

            // Skip packet.
            packet.copy_within(length.., 0);
            packet.truncate(packet.len() - length);
            target_len = None;
        }
    }
}

pub fn encrypt_data(cipher: &mut Aes256Ctr, data: &mut Vec<u8>) {
    let len = data.len();

    data.reserve(len + 68);
    data.resize(len + 36, 0);
    data.copy_within(..len, 36);
    data[..4].copy_from_slice(&((len + 64) as u32).to_le_bytes());

    let mut nonce = [0u8; 32];
    rand::thread_rng().fill(&mut nonce[..]);

    data.extend_from_slice(sha2::Sha256::digest(&data[4..]).as_slice());

    cipher.apply_keystream(data);
}

pub fn build_handshake_packet(
    server_pubkey: &ed25519::PublicKey,
    client_secret: &ed25519::SecretKey,
    buffer: &mut Vec<u8>,
) {
    let server_short_id = tl_proto::hash(server_pubkey.as_tl());
    let client_public_key = ed25519::PublicKey::from(client_secret);

    let shared_secret = client_secret.expand().compute_shared_secret(server_pubkey);

    // Prepare packet
    let checksum: [u8; 32] = sha2::Sha256::digest(buffer.as_slice()).into();

    let length = buffer.len();
    buffer.resize(length + 96, 0);
    buffer.copy_within(..length, 96);

    buffer[..32].copy_from_slice(server_short_id.as_slice());
    buffer[32..64].copy_from_slice(client_public_key.as_bytes());
    buffer[64..96].copy_from_slice(&checksum);

    // Encrypt packet data
    build_packet_cipher(&shared_secret, &checksum).apply_keystream(&mut buffer[96..]);
}

pub fn build_packet_cipher(shared_secret: &[u8; 32], checksum: &[u8; 32]) -> Aes256Ctr {
    let mut aes_key_bytes: [u8; 32] = *shared_secret;
    aes_key_bytes[16..32].copy_from_slice(&checksum[16..32]);
    let mut aes_ctr_bytes: [u8; 16] = checksum[0..16].try_into().unwrap();
    aes_ctr_bytes[4..16].copy_from_slice(&shared_secret[20..32]);

    Aes256Ctr::new(
        &generic_array::GenericArray::from(aes_key_bytes),
        &generic_array::GenericArray::from(aes_ctr_bytes),
    )
}

#[derive(thiserror::Error, Debug)]
pub enum TcpAdnlError {
    #[error("failed to open connection")]
    ConnectionError(#[source] std::io::Error),
    #[error("socket closed")]
    SocketClosed,
    #[error("invalid answer")]
    InvalidAnswer(#[source] tl_proto::TlError),
    #[error("duplicate query")]
    DuplicateQuery,
}

pub type Aes256Ctr = ctr::Ctr64BE<aes::Aes256>;
