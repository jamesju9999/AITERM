//! CDP 連線與寫入閘門。用 `Target.setAutoAttach`（flatten）附加到每個分頁，對它啟用
//! `Fetch`，每個請求先停在這裡，交給 `gate::decide` 判定：放行、拒絕，或等使用者核准。
//! 探勘結果見 docs/superpowers/specs/2026-11-17-system-connections-design.md。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use super::approvals::{Approvals, Outcome, Pending, APPROVAL_TIMEOUT};
use super::audit::{self, Entry, Verdict};
use super::gate::decide;
use super::types::{Decision, DenyReason, SystemConnection};

/// 閘門發給畫面的事件。
#[derive(Debug, Clone)]
pub enum GateEvent {
    NeedApproval { id: String, pending: Pending },
    Resolved { id: String },
    Denied { system_id: String, method: String, url: String, reason: DenyReason },
}

/// 一條 CDP 連線：送指令、等回應、收事件。
pub struct Cdp {
    tx: mpsc::UnboundedSender<String>,
    next_id: AtomicU64,
    waiters: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
}

impl Cdp {
    /// 連上並回傳（連線物件，事件接收端）。事件是 `{method, params, sessionId?}`。
    pub async fn connect(ws_url: &str) -> Result<(Arc<Cdp>, mpsc::UnboundedReceiver<Value>), String> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url).await.map_err(|e| format!("CDP 連線失敗：{e}"))?;
        let (mut sink, mut stream) = ws.split();
        let (tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel::<Value>();
        let waiters: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> = Arc::new(Mutex::new(HashMap::new()));
        tokio::spawn(async move {
            while let Some(m) = out_rx.recv().await {
                if sink.send(Message::text(m)).await.is_err() {
                    break;
                }
            }
        });
        let w = waiters.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let Message::Text(t) = msg else { continue };
                let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                if let Some(id) = v["id"].as_u64() {
                    if let Some(tx) = w.lock().remove(&id) {
                        let _ = tx.send(v);
                    }
                } else if ev_tx.send(v).is_err() {
                    break;
                }
            }
        });
        Ok((Arc::new(Cdp { tx, next_id: AtomicU64::new(1), waiters }), ev_rx))
    }

    pub async fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().insert(id, tx);
        self.send(id, method, params, session)?;
        let v = tokio::time::timeout(Duration::from_secs(15), rx)
            .await
            .map_err(|_| format!("CDP {method} 逾時"))?
            .map_err(|_| "CDP 連線已關閉".to_string())?;
        if let Some(e) = v.get("error") {
            return Err(format!("CDP {method} 失敗：{e}"));
        }
        Ok(v["result"].clone())
    }

    /// 不等回應（`Fetch.continueRequest` 這類）。
    pub fn fire(&self, method: &str, params: Value, session: Option<&str>) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let _ = self.send(id, method, params, session);
    }

    fn send(&self, id: u64, method: &str, params: Value, session: Option<&str>) -> Result<(), String> {
        let mut m = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            m["sessionId"] = json!(s);
        }
        self.tx.send(m.to_string()).map_err(|_| "CDP 連線已關閉".to_string())
    }
}

/// 閘門需要的外部資源。
#[derive(Clone)]
pub struct GateCtx {
    pub conn: Arc<Mutex<SystemConnection>>,
    pub approvals: Arc<Approvals>,
    pub audit: Option<SqlitePool>,
    /// AI 最近一次宣告的意圖（`proxy` 在轉發工具呼叫前寫入）。
    pub intent: Arc<Mutex<String>>,
    /// 使用者接手中：不判定、直接放行，仍寫審計。
    pub manual: Arc<AtomicBool>,
    pub emit: Arc<dyn Fn(GateEvent) + Send + Sync>,
    pub approval_timeout: Duration,
}

impl GateCtx {
    pub fn new(conn: SystemConnection, approvals: Arc<Approvals>, emit: Arc<dyn Fn(GateEvent) + Send + Sync>) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
            approvals,
            audit: None,
            intent: Arc::new(Mutex::new(String::new())),
            manual: Arc::new(AtomicBool::new(false)),
            emit,
            approval_timeout: APPROVAL_TIMEOUT,
        }
    }
}

fn declared_read(intent: &str) -> bool {
    let i = intent.trim_start().to_lowercase();
    i.starts_with('讀') || i.starts_with("read")
}

async fn write_audit(ctx: &GateCtx, method: &str, url: &str, verdict: Verdict, source: &str) {
    let Some(pool) = &ctx.audit else { return };
    let system_id = ctx.conn.lock().id.clone();
    let intent = ctx.intent.lock().clone();
    let e = Entry::new(&system_id, "gate", &intent, Some(method), Some(url), verdict, source);
    if let Err(err) = audit::record(pool, &e).await {
        log::warn!("systems audit: {err}");
    }
}

/// 連上 Chrome，對每個分頁攔截請求，直到連線中斷。
pub async fn run_gate(ws_url: &str, ctx: GateCtx) -> Result<(), String> {
    let (cdp, mut events) = Cdp::connect(ws_url).await?;
    cdp.call("Target.setDiscoverTargets", json!({ "discover": true }), None).await?;
    cdp.call(
        "Target.setAutoAttach",
        json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }),
        None,
    )
    .await?;
    // sessionId → targetId（只有分頁；主框架的 frameId 等於 targetId）。每個 target 只附加一次。
    let sessions: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let seen: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    while let Some(ev) = events.recv().await {
        let method = ev["method"].as_str().unwrap_or("").to_string();
        match method.as_str() {
            "Target.attachedToTarget" => {
                let info = &ev["params"]["targetInfo"];
                let sid = ev["params"]["sessionId"].as_str().unwrap_or("").to_string();
                if sid.is_empty() {
                    continue;
                }
                if info["type"] != "page" {
                    // waitForDebuggerOnStart 讓每個新 target 都停在原地等我們放行。非分頁的（擴充功能的
                    // service worker、瀏覽器內部頁面…）我們不攔，但一定要放行，否則 Playwright 連瀏覽器時
                    // 會一直等那些 target，整整 30 秒後逾時（有頭模式的真機才會遇到，無頭測試沒有這些）。
                    let cdp = cdp.clone();
                    tokio::spawn(async move {
                        let _ = cdp.call("Runtime.runIfWaitingForDebugger", json!({}), Some(&sid)).await;
                    });
                    continue;
                }
                let tid = info["targetId"].as_str().unwrap_or("").to_string();
                if !seen.lock().insert(tid.clone()) {
                    continue;
                }
                sessions.lock().insert(sid.clone(), tid);
                let cdp = cdp.clone();
                tokio::spawn(async move {
                    let _ = cdp.call("Fetch.enable", json!({ "patterns": [{ "urlPattern": "*" }] }), Some(&sid)).await;
                    let _ = cdp.call("Runtime.runIfWaitingForDebugger", json!({}), Some(&sid)).await;
                });
            }
            "Target.detachedFromTarget" => {
                if let Some(sid) = ev["params"]["sessionId"].as_str() {
                    if let Some(tid) = sessions.lock().remove(sid) {
                        seen.lock().remove(&tid);
                    }
                }
            }
            "Fetch.requestPaused" => {
                let Some(sid) = ev["sessionId"].as_str().map(String::from) else { continue };
                let top_frame = sessions.lock().get(&sid).cloned();
                let cdp = cdp.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move { handle_paused(cdp, ctx, sid, top_frame, ev["params"].clone()).await });
            }
            _ => {}
        }
    }
    Ok(())
}

async fn handle_paused(cdp: Arc<Cdp>, ctx: GateCtx, sid: String, top_frame: Option<String>, p: Value) {
    let request_id = p["requestId"].as_str().unwrap_or("").to_string();
    let method = p["request"]["method"].as_str().unwrap_or("GET").to_string();
    let url = p["request"]["url"].as_str().unwrap_or("").to_string();
    let is_nav = p["resourceType"] == "Document" && p["frameId"].as_str().is_some() && p["frameId"].as_str() == top_frame.as_deref();
    let is_write = !matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD" | "OPTIONS");

    let proceed = |cdp: &Cdp| cdp.fire("Fetch.continueRequest", json!({ "requestId": request_id }), Some(&sid));
    let block = |cdp: &Cdp| {
        cdp.fire("Fetch.failRequest", json!({ "requestId": request_id, "errorReason": "BlockedByClient" }), Some(&sid))
    };

    if ctx.manual.load(Ordering::SeqCst) {
        proceed(&cdp);
        if is_write {
            write_audit(&ctx, &method, &url, Verdict::Manual, "user").await;
        }
        return;
    }
    let (decision, system_id) = {
        let c = ctx.conn.lock();
        (decide(&c, &method, &url, is_nav), c.id.clone())
    };
    match decision {
        Decision::Allow => {
            proceed(&cdp);
            if is_write {
                write_audit(&ctx, &method, &url, Verdict::Allow, "rule").await;
            }
        }
        Decision::Deny(reason) => {
            block(&cdp);
            write_audit(&ctx, &method, &url, Verdict::Deny, "rule").await;
            (ctx.emit)(GateEvent::Denied { system_id, method, url, reason });
        }
        Decision::Ask => {
            let intent = ctx.intent.lock().clone();
            let pending = Pending {
                system_id,
                declared_read: declared_read(&intent),
                intent,
                method: method.clone(),
                url: url.clone(),
                ts_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            };
            let (id, rx) = ctx.approvals.open(pending.clone());
            (ctx.emit)(GateEvent::NeedApproval { id: id.clone(), pending });
            let outcome = Approvals::wait(rx, ctx.approval_timeout).await;
            ctx.approvals.expire(&id);
            (ctx.emit)(GateEvent::Resolved { id });
            match outcome {
                Outcome::Approved => {
                    proceed(&cdp);
                    write_audit(&ctx, &method, &url, Verdict::Approved, "user").await;
                }
                Outcome::Rejected => {
                    block(&cdp);
                    write_audit(&ctx, &method, &url, Verdict::Rejected, "user").await;
                }
                Outcome::TimedOut => {
                    block(&cdp);
                    write_audit(&ctx, &method, &url, Verdict::Timeout, "rule").await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser;
    use crate::types::Mode;
    use crate::testsite::{conn, fake_site};
    use std::sync::atomic::AtomicUsize;

    /// 開真的 Chrome、跑閘門、導覽到假網站並送出表單；回傳（POST 次數，核准事件的 id 清單）。
    async fn submit_form(mode: Mode, decide_with: Option<bool>) -> (usize, usize) {
        let (port, posts) = fake_site().await;
        let dir = tempfile::tempdir().unwrap();
        let mut b = browser::launch(dir.path(), "t", "about:blank").await.unwrap();
        let approvals = Arc::new(Approvals::new());
        let asked = Arc::new(AtomicUsize::new(0));
        let (a2, ap2) = (asked.clone(), approvals.clone());
        let emit: Arc<dyn Fn(GateEvent) + Send + Sync> = Arc::new(move |e| {
            if let GateEvent::NeedApproval { id, .. } = e {
                a2.fetch_add(1, Ordering::SeqCst);
                if let Some(ok) = decide_with {
                    let ap = ap2.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        ap.decide(&id, ok);
                    });
                }
            }
        });
        let mut ctx = GateCtx::new(conn(port, mode), approvals, emit);
        ctx.approval_timeout = Duration::from_secs(3);
        let ws = b.ws_url.clone();
        let gate = tokio::spawn(async move { run_gate(&ws, ctx).await });
        tokio::time::sleep(Duration::from_millis(800)).await;

        // 測試自己的第三個用戶端：開分頁、載入假網站、送出表單。
        let (cdp, _ev) = Cdp::connect(&b.ws_url).await.unwrap();
        let t = cdp.call("Target.createTarget", json!({ "url": format!("http://127.0.0.1:{port}/") }), None).await.unwrap();
        let tid = t["targetId"].as_str().unwrap().to_string();
        let s = cdp.call("Target.attachToTarget", json!({ "targetId": tid, "flatten": true }), None).await.unwrap();
        let sid = s["sessionId"].as_str().unwrap().to_string();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let _ = cdp.call("Runtime.evaluate", json!({ "expression": "document.forms[0].submit()" }), Some(&sid)).await;
        tokio::time::sleep(Duration::from_millis(4500)).await;

        let n = posts.load(Ordering::SeqCst);
        let asked = asked.load(Ordering::SeqCst);
        gate.abort();
        b.close().await;
        (n, asked)
    }

    /// 最重要的一條：被拒絕或沒核准的寫入，伺服器真的沒收到。需要真的 Chrome：
    /// `cargo test -p app --lib -- --ignored --test-threads=1 systems::cdp`
    #[tokio::test]
    #[ignore]
    async fn read_only_blocks_the_write_and_autonomous_lets_it_through() {
        let (n, _) = submit_form(Mode::ReadOnly, None).await;
        assert_eq!(n, 0, "read_only：伺服器不該收到 POST");
        let (n, _) = submit_form(Mode::Autonomous, None).await;
        assert_eq!(n, 1, "autonomous：伺服器收到一次");
    }

    #[tokio::test]
    #[ignore]
    async fn approve_each_waits_for_the_user_and_honours_the_decision() {
        let (n, asked) = submit_form(Mode::ApproveEach, Some(false)).await;
        assert_eq!((n, asked), (0, 1), "拒絕：問了一次、伺服器沒收到");
        let (n, asked) = submit_form(Mode::ApproveEach, Some(true)).await;
        assert_eq!((n, asked), (1, 1), "核准：問了一次、伺服器收到一次");
        let (n, asked) = submit_form(Mode::ApproveEach, None).await;
        assert_eq!((n, asked), (0, 1), "沒人回應：逾時視為拒絕");
    }

    #[test]
    fn a_read_intent_is_recognised_in_both_languages() {
        assert!(declared_read("讀：看訂單清單"));
        assert!(declared_read("  Read the order list"));
        assert!(!declared_read("寫：送出訂單"));
    }
}
