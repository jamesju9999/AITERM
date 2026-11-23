//! 共用狀態與通知。
use crate::db::Pools;
use crate::secrets::FileSecrets;
use aiterm_core::share::backoff::AuthBackoff;
use aiterm_intake_core::host::Notifier;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// 資料變動通知：遞增 `seq` 並推播給所有 WebSocket 連線。
pub struct Broadcast {
    seq: AtomicU64,
    tx: broadcast::Sender<u64>,
}

impl Broadcast {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(64);
        Arc::new(Self { seq: AtomicU64::new(1), tx })
    }
    pub fn seq(&self) -> u64 {
        self.seq.load(Ordering::SeqCst)
    }
    pub fn subscribe(&self) -> broadcast::Receiver<u64> {
        self.tx.subscribe()
    }
}

impl Notifier for Broadcast {
    fn changed(&self) {
        let n = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.tx.send(n);
    }
}

pub struct AppState {
    pub pools: Pools,
    pub secrets: Arc<FileSecrets>,
    pub events: Arc<Broadcast>,
    pub fingerprint: String,
    pub data_dir: PathBuf,
    pub backoff: AuthBackoff,
    pub version: String,
}

impl AppState {
    pub fn new(pools: Pools, secrets: Arc<FileSecrets>, fingerprint: String, data_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self { pools, secrets, events: Broadcast::new(), fingerprint, data_dir, backoff: AuthBackoff::new(), version: env!("CARGO_PKG_VERSION").to_string() })
    }
}
