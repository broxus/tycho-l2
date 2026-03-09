use std::sync::Arc;

use arc_swap::ArcSwap;
use tycho_types::num::Tokens;

pub const LABEL_SRC: &str = "src";
pub const LABEL_DST: &str = "dst";
pub const LABEL_WALLET: &str = "wallet";

const METRIC_STATUS: &str = "sync_uploader_status";
const METRIC_WALLET_BALANCE: &str = "sync_uploader_wallet_balance";
const METRIC_WALLET_MIN_REQUIRED_BALANCE: &str = "sync_uploader_wallet_min_required_balance";
const METRIC_LAST_CHECKED_VSET: &str = "sync_uploader_last_checked_vset";
const METRIC_MIN_BRIDGE_STATE_LT: &str = "sync_uploader_min_bridge_state_lt";
const METRIC_CACHED_KEY_BLOCKS: &str = "sync_uploader_cached_key_blocks";
const METRIC_LAST_SEEN_SRC_KEY_BLOCK_SEQNO: &str = "sync_uploader_last_seen_src_key_block_seqno";
const METRIC_LAST_SENT_KEY_BLOCK_SEQNO: &str = "sync_uploader_last_sent_key_block_seqno";
const METRIC_LAST_SENT_KEY_BLOCK_UTIME: &str = "sync_uploader_last_sent_key_block_utime";
const METRIC_LAST_SUCCESS_UNIX_TIME: &str = "sync_uploader_last_success_unix_time";
const METRIC_LAST_ERROR_UNIX_TIME: &str = "sync_uploader_last_error_unix_time";

#[derive(Debug, Clone, Copy, Default)]
pub struct UploaderMetricsSnapshot {
    pub status: UploaderStatus,
    pub wallet_balance: Tokens,
    pub wallet_min_required_balance: Tokens,
    pub last_checked_vset: u32,
    pub min_bridge_state_lt: u64,
    pub cached_key_blocks: usize,
    pub last_seen_src_key_block_seqno: u32,
    pub last_sent_key_block_seqno: u32,
    pub last_sent_key_block_utime: u32,
    pub last_success_unix_time: u64,
    pub last_error_unix_time: u64,
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(u8)]
pub enum UploaderStatus {
    #[default]
    Initializing = 0,
    Running = 1,
    Retrying = 2,
}

pub struct UploaderMetricsState {
    src: String,
    dst: String,
    wallet: String,
    snapshot: ArcSwap<UploaderMetricsSnapshot>,
}

impl UploaderMetricsState {
    pub fn new(
        src: impl Into<String>,
        dst: impl Into<String>,
        wallet: impl Into<String>,
    ) -> Arc<Self> {
        let state = Arc::new(Self {
            src: src.into(),
            dst: dst.into(),
            wallet: wallet.into(),
            snapshot: ArcSwap::new(Arc::new(UploaderMetricsSnapshot::default())),
        });
        state.publish_snapshot(UploaderMetricsSnapshot::default());
        state
    }

    pub fn update(&self, update: impl FnOnce(&mut UploaderMetricsSnapshot)) {
        let mut snapshot = *self.snapshot.load_full();
        update(&mut snapshot);
        self.snapshot.store(Arc::new(snapshot));
        self.publish_snapshot(snapshot);
    }

    pub fn snapshot(&self) -> UploaderMetricsSnapshot {
        *self.snapshot.load_full()
    }

    fn publish_snapshot(&self, snapshot: UploaderMetricsSnapshot) {
        let labels = [(LABEL_SRC, self.src.clone()), (LABEL_DST, self.dst.clone())];
        let wallet_labels = [
            (LABEL_SRC, self.src.clone()),
            (LABEL_DST, self.dst.clone()),
            (LABEL_WALLET, self.wallet.clone()),
        ];

        metrics::gauge!(METRIC_STATUS, &labels).set(snapshot.status as u8);
        metrics::gauge!(METRIC_WALLET_BALANCE, &wallet_labels)
            .set(snapshot.wallet_balance.into_inner() as f64);
        metrics::gauge!(METRIC_WALLET_MIN_REQUIRED_BALANCE, &wallet_labels)
            .set(snapshot.wallet_min_required_balance.into_inner() as f64);
        metrics::gauge!(METRIC_LAST_CHECKED_VSET, &labels).set(snapshot.last_checked_vset as f64);
        metrics::gauge!(METRIC_MIN_BRIDGE_STATE_LT, &labels)
            .set(snapshot.min_bridge_state_lt as f64);
        metrics::gauge!(METRIC_CACHED_KEY_BLOCKS, &labels).set(snapshot.cached_key_blocks as f64);
        metrics::gauge!(METRIC_LAST_SEEN_SRC_KEY_BLOCK_SEQNO, &labels)
            .set(snapshot.last_seen_src_key_block_seqno as f64);
        metrics::gauge!(METRIC_LAST_SENT_KEY_BLOCK_SEQNO, &labels)
            .set(snapshot.last_sent_key_block_seqno as f64);
        metrics::gauge!(METRIC_LAST_SENT_KEY_BLOCK_UTIME, &labels)
            .set(snapshot.last_sent_key_block_utime as f64);
        metrics::gauge!(METRIC_LAST_SUCCESS_UNIX_TIME, &labels)
            .set(snapshot.last_success_unix_time as f64);
        metrics::gauge!(METRIC_LAST_ERROR_UNIX_TIME, &labels)
            .set(snapshot.last_error_unix_time as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_snapshot_values() {
        let state = UploaderMetricsState::new("ton_testnet", "tycho_devnet1", "0:wallet");
        state.update(|snapshot| {
            snapshot.status = UploaderStatus::Running;
            snapshot.wallet_balance = Tokens::new(321);
            snapshot.wallet_min_required_balance = Tokens::new(100);
            snapshot.last_checked_vset = 11;
            snapshot.cached_key_blocks = 3;
            snapshot.last_success_unix_time = 123;
        });

        let snapshot = state.snapshot();
        assert!(matches!(snapshot.status, UploaderStatus::Running));
        assert_eq!(snapshot.wallet_balance, Tokens::new(321));
        assert_eq!(snapshot.wallet_min_required_balance, Tokens::new(100));
        assert_eq!(snapshot.last_checked_vset, 11);
        assert_eq!(snapshot.cached_key_blocks, 3);
        assert_eq!(snapshot.last_success_unix_time, 123);
    }
}
