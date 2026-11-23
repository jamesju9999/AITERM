//! 整合測試用：在隨機埠起一個真的 TLS server（記憶體資料庫、暫存資料夾）。
use crate::db;
use crate::members::{self, Role};
use crate::secrets::FileSecrets;
use crate::server::{self, Running};
use crate::state::AppState;
use std::sync::Arc;

pub struct TestServer {
    pub running: Running,
    pub dir: tempfile::TempDir,
}

impl TestServer {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let identity = crate::tls::load_or_generate(dir.path(), vec!["127.0.0.1".into()]).unwrap();
        let pools = db::open(dir.path()).await.unwrap();
        let secrets = Arc::new(FileSecrets::open(dir.path()).unwrap());
        let state = AppState::new(pools, secrets, identity.fingerprint(), dir.path().to_path_buf());
        let running = server::start("127.0.0.1:0".parse().unwrap(), state, identity).await.unwrap();
        Self { running, dir }
    }
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.running.addr.port())
    }
    pub fn state(&self) -> &Arc<AppState> {
        &self.running.state
    }
    /// 直接建一個邀請，回傳邀請碼。
    pub async fn invite(&self, name: &str, role: Role) -> String {
        members::create_invite(&self.state().pools.server, name, role, db::now()).await.unwrap().code
    }
}
