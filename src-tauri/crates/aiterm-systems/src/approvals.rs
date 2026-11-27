//! 等待使用者核准的寫入請求。閘門（`cdp`）把非 GET 請求交進來，使用者在畫面按核准／拒絕，
//! 或逾時（視為拒絕）。

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::oneshot;

/// 沒有人回應時多久後自動拒絕。
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pending {
    pub system_id: String,
    pub intent: String,
    pub method: String,
    pub url: String,
    /// AI 宣告成「讀」，卻觸發了寫入：核准卡要標警示。
    pub declared_read: bool,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Approved,
    Rejected,
    TimedOut,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingView {
    pub id: String,
    #[serde(flatten)]
    pub pending: Pending,
}

#[derive(Default)]
pub struct Approvals {
    inner: Mutex<HashMap<String, (Pending, oneshot::Sender<Outcome>)>>,
}

impl Approvals {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&self, pending: Pending) -> (String, oneshot::Receiver<Outcome>) {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.inner.lock().insert(id.clone(), (pending, tx));
        (id, rx)
    }

    /// 使用者決定。回傳這個 id 是否還在等（已逾時或已處理過就是 false）。
    pub fn decide(&self, id: &str, approve: bool) -> bool {
        match self.inner.lock().remove(id) {
            Some((_, tx)) => {
                let _ = tx.send(if approve { Outcome::Approved } else { Outcome::Rejected });
                true
            }
            None => false,
        }
    }

    /// 這個系統所有等待中的都拒絕（停止、停用時用）。回傳拒絕了幾筆。
    pub fn reject_all(&self, system_id: &str) -> usize {
        let mut map = self.inner.lock();
        let ids: Vec<String> = map.iter().filter(|(_, (p, _))| p.system_id == system_id).map(|(k, _)| k.clone()).collect();
        for id in &ids {
            if let Some((_, tx)) = map.remove(id) {
                let _ = tx.send(Outcome::Rejected);
            }
        }
        ids.len()
    }

    pub fn has_pending(&self, system_id: &str) -> bool {
        self.inner.lock().values().any(|(p, _)| p.system_id == system_id)
    }

    pub fn list(&self) -> Vec<PendingView> {
        let mut v: Vec<PendingView> =
            self.inner.lock().iter().map(|(id, (p, _))| PendingView { id: id.clone(), pending: p.clone() }).collect();
        v.sort_by_key(|x| x.pending.ts_ms);
        v
    }

    /// 等結果；逾時回 `TimedOut`。
    pub async fn wait(rx: oneshot::Receiver<Outcome>, timeout: Duration) -> Outcome {
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(o)) => o,
            Ok(Err(_)) => Outcome::Rejected, // 送端被丟掉（系統被關閉）
            Err(_) => Outcome::TimedOut,
        }
    }

    /// 逾時後把這筆從清單移掉，免得畫面一直留著已經失效的核准卡。
    pub fn expire(&self, id: &str) {
        self.inner.lock().remove(id);
    }
}

impl Pending {
    #[cfg(test)]
    pub fn sample(system_id: &str) -> Self {
        Self {
            system_id: system_id.into(),
            intent: "寫：送出表單".into(),
            method: "POST".into(),
            url: "https://x/submit".into(),
            declared_read: false,
            ts_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_pending_approval_resolves_with_the_users_decision() {
        let a = Approvals::new();
        let (id, rx) = a.open(Pending::sample("s1"));
        assert!(a.decide(&id, true));
        assert_eq!(rx.await.unwrap(), Outcome::Approved);
        assert!(!a.decide(&id, false), "已處理過的 id 不能再決定一次");
    }

    #[tokio::test]
    async fn an_unanswered_approval_times_out_and_can_be_expired() {
        let a = Approvals::new();
        let (id, rx) = a.open(Pending::sample("s1"));
        let out = Approvals::wait(rx, Duration::from_millis(30)).await;
        assert_eq!(out, Outcome::TimedOut);
        a.expire(&id);
        assert!(!a.has_pending("s1"));
    }

    #[tokio::test]
    async fn reject_all_only_touches_that_system() {
        let a = Approvals::new();
        let (_i1, r1) = a.open(Pending::sample("s1"));
        let (_i2, _r2) = a.open(Pending::sample("s2"));
        assert_eq!(a.reject_all("s1"), 1);
        assert_eq!(r1.await.unwrap(), Outcome::Rejected);
        assert!(a.has_pending("s2"));
        assert!(!a.has_pending("s1"));
    }

    #[test]
    fn list_is_oldest_first() {
        let a = Approvals::new();
        let mut p = Pending::sample("s1");
        p.ts_ms = 5;
        let (_a, _r1) = a.open(p.clone());
        p.ts_ms = 2;
        let (_b, _r2) = a.open(p);
        let l = a.list();
        assert_eq!(l[0].pending.ts_ms, 2);
    }
}
