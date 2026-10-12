//! 向使用者要授權。`Approver` 是介面；真實實作靠事件＋oneshot 跟前端對話。

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

/// 前端監聽這個事件顯示授權對話框。
pub const APPROVAL_REQUEST_EVENT: &str = "computer://approval-request";
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    /// 第一次操作這個 App。
    FirstUse,
    /// 破壞性操作，每次都問。
    Destructive,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    pub kind: ApprovalKind,
    pub app: String,
    pub action: String,
    pub detail: String,
}

#[async_trait]
pub trait Approver: Send + Sync {
    async fn ask(&self, req: &ApprovalRequest) -> Decision;
}

#[derive(Serialize, Clone)]
struct ApprovalEvent<'a> {
    id: &'a str,
    #[serde(flatten)]
    req: &'a ApprovalRequest,
}

pub struct TauriApprover {
    app: AppHandle,
    pending: Mutex<HashMap<String, oneshot::Sender<Decision>>>,
}

impl TauriApprover {
    pub fn new(app: AppHandle) -> Self {
        Self { app, pending: Mutex::new(HashMap::new()) }
    }

    /// 前端回覆。id 不存在（已逾時）回 false。
    pub fn resolve(&self, id: &str, decision: Decision) -> bool {
        match self.pending.lock().remove(id) {
            Some(tx) => tx.send(decision).is_ok(),
            None => false,
        }
    }
}

#[async_trait]
impl Approver for TauriApprover {
    async fn ask(&self, req: &ApprovalRequest) -> Decision {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id.clone(), tx);
        // 對話框畫在視窗裡；請求多半來自外部 MCP 客戶端，視窗可能藏著。
        crate::resident::show_main_window(&self.app);
        if self.app.emit(APPROVAL_REQUEST_EVENT, ApprovalEvent { id: &id, req }).is_err() {
            self.pending.lock().remove(&id);
            return Decision::Deny;
        }
        let decision = match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
            Ok(Ok(d)) => d,
            _ => Decision::Deny, // 逾時或前端沒回：視為拒絕
        };
        self.pending.lock().remove(&id);
        decision
    }
}
