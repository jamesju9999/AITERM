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
    pub ai: Arc<crate::ai::ServerAi>,
    pub channels: crate::channels::Channels,
    /// 收訊管道（LINE webhook）綁定的位址；預設只收本機（前面通常有反向代理或 Tunnel）。
    pub channel_bind: std::net::IpAddr,
    pub events: Arc<Broadcast>,
    pub fingerprint: String,
    pub data_dir: PathBuf,
    pub backoff: AuthBackoff,
    pub version: String,
    /// LINE API 位址；測試時指向假的 server。
    pub line_api_base: parking_lot::Mutex<String>,
}

impl AppState {
    pub fn with_channel_bind(pools: Pools, secrets: Arc<FileSecrets>, fingerprint: String, data_dir: PathBuf, channel_bind: std::net::IpAddr) -> Arc<Self> {
        let me = Self::new(pools, secrets, fingerprint, data_dir);
        // `new` 回傳 Arc；剛建立時只有一個參照，可以直接改。
        let mut me = Arc::try_unwrap(me).ok().expect("剛建立的 AppState 只有一個參照");
        me.channel_bind = channel_bind;
        Arc::new(me)
    }

    pub fn new(pools: Pools, secrets: Arc<FileSecrets>, fingerprint: String, data_dir: PathBuf) -> Arc<Self> {
        let ai = Arc::new(crate::ai::ServerAi::new(pools.intake.clone(), secrets.clone()));
        Arc::new(Self { pools, secrets, ai, channels: Default::default(), channel_bind: std::net::Ipv4Addr::LOCALHOST.into(), events: Broadcast::new(), fingerprint, data_dir, backoff: AuthBackoff::new(), version: env!("CARGO_PKG_VERSION").to_string(), line_api_base: parking_lot::Mutex::new(aiterm_intake_core::line_server::API_BASE.to_string()) })
    }
}
