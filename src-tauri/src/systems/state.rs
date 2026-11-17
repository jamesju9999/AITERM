//! 系統連線的執行狀態：每個系統一個瀏覽器＋閘門＋代理，以及停止、接手、停用的控制。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::approvals::{Approvals, PendingView};
use super::audit;
use super::browser::{self, RunningBrowser};
use super::cdp::{run_gate, GateCtx, GateEvent};
use super::gate::host_of;
use super::proxy::{fetch_catalog, Proxy, Upstream};
use super::types::SystemConnection;
use crate::config::ConfigStore;

/// 畫面事件的送出函式：`(事件名, 內容)`。
pub type Emitter = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub struct Running {
    browser: tokio::sync::Mutex<RunningBrowser>,
    gate: tokio::task::JoinHandle<()>,
    pub proxy: Arc<Proxy>,
    pub ctx: GateCtx,
    needs_login: AtomicBool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemStatus {
    pub running: bool,
    pub manual: bool,
    pub pending: usize,
    pub needs_login: bool,
}

pub struct SystemsState {
    config: Arc<ConfigStore>,
    pub approvals: Arc<Approvals>,
    audit: tokio::sync::OnceCell<Option<SqlitePool>>,
    running: tokio::sync::Mutex<HashMap<String, Arc<Running>>>,
    emitter: Mutex<Option<Emitter>>,
    catalog: tokio::sync::OnceCell<Vec<Value>>,
    data_dir: PathBuf,
}

/// 頁面看起來被導去登入頁（登入失效）：網址不在白名單，或路徑像登入頁。
pub fn looks_logged_out(conn: &SystemConnection, tool_text: &str) -> bool {
    let Some(url) = tool_text.split("Page URL: ").nth(1).and_then(|s| s.split_whitespace().next()) else {
        return false;
    };
    let on_allowed = conn.allowed_hosts.iter().any(|a| {
        let a = a.trim().trim_start_matches("*.").to_ascii_lowercase();
        host_of(url).is_some_and(|h| h == a || h.ends_with(&format!(".{a}")))
    });
    if !on_allowed {
        return true;
    }
    let path = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let path = path.split_once('/').map(|(_, p)| p).unwrap_or("").to_ascii_lowercase();
    ["login", "signin", "sign-in", "sso", "oauth"].iter().any(|k| path.contains(k))
}

impl SystemsState {
    pub fn new(config: Arc<ConfigStore>) -> Self {
        let data_dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM");
        Self {
            config,
            approvals: Arc::new(Approvals::new()),
            audit: tokio::sync::OnceCell::new(),
            running: tokio::sync::Mutex::new(HashMap::new()),
            emitter: Mutex::new(None),
            catalog: tokio::sync::OnceCell::new(),
            data_dir,
        }
    }

    pub fn set_emitter(&self, e: Emitter) {
        *self.emitter.lock() = Some(e);
    }

    fn emit(&self, name: &str, payload: Value) {
        if let Some(e) = self.emitter.lock().as_ref() {
            e(name, payload);
        }
    }

    pub async fn audit_pool(&self) -> Option<SqlitePool> {
        self.audit
            .get_or_init(|| async {
                match audit::open().await {
                    Ok(p) => Some(p),
                    Err(e) => {
                        log::error!("systems.db 開啟失敗，審計記錄停用：{e}");
                        None
                    }
                }
            })
            .await
            .clone()
    }

    pub fn connections(&self) -> Vec<SystemConnection> {
        self.config.get().system_connections
    }

    #[cfg(test)]
    pub fn set_catalog_for_test(&self, tools: Vec<Value>) {
        let _ = self.catalog.set(tools);
    }

    pub fn connection(&self, id: &str) -> Result<SystemConnection, String> {
        self.config
            .get()
            .system_connections
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("找不到系統連線：{id}"))
    }

    /// 對外的工具清單（只取一次）。
    pub async fn catalog(&self) -> Result<Vec<Value>, String> {
        self.catalog
            .get_or_try_init(|| async {
                let out = self.data_dir.join("system-output").join("_catalog");
                std::fs::create_dir_all(&out).ok();
                fetch_catalog(&out).await
            })
            .await
            .cloned()
    }

    /// 這個系統的瀏覽器、閘門、代理都就緒；沒開就開。
    pub async fn ensure_running(self: &Arc<Self>, id: &str) -> Result<Arc<Running>, String> {
        let conn = self.connection(id)?;
        if !conn.enabled {
            return Err("這個系統連線已停用".into());
        }
        let mut map = self.running.lock().await;
        if let Some(r) = map.get(id) {
            let alive = r.browser.lock().await.is_alive() && !r.gate.is_finished();
            if alive {
                return Ok(r.clone());
            }
            r.proxy.close().await;
            r.gate.abort();
            map.remove(id);
        }

        let browser = browser::launch(&self.data_dir, id, &conn.start_url).await?;
        let last_denied: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let me = self.clone();
        let ld = last_denied.clone();
        let emit: Arc<dyn Fn(GateEvent) + Send + Sync> = Arc::new(move |e| match e {
            GateEvent::NeedApproval { id, pending } => {
                me.emit("systems://need-approval", json!({ "id": id, "pending": pending }));
            }
            GateEvent::Resolved { id } => me.emit("systems://resolved", json!({ "id": id })),
            GateEvent::Denied { system_id, method, url, reason } => {
                *ld.lock() = Some(format!("{method} {url}"));
                me.emit("systems://denied", json!({ "system_id": system_id, "method": method, "url": url, "reason": reason }));
            }
        });
        let mut ctx = GateCtx::new(conn, self.approvals.clone(), emit);
        ctx.audit = self.audit_pool().await;
        let ws = browser.ws_url.clone();
        let gate_ctx = ctx.clone();
        let gate = tokio::spawn(async move {
            if let Err(e) = run_gate(&ws, gate_ctx).await {
                log::warn!("systems gate 結束：{e}");
            }
        });
        tokio::time::sleep(Duration::from_millis(600)).await;

        let out = self.data_dir.join("system-output").join(id);
        std::fs::create_dir_all(&out).ok();
        let upstream = Upstream::spawn(Some(&browser.cdp_http()), &out).await?;
        let proxy = Proxy::start(id, upstream, self.approvals.clone(), ctx.intent.clone(), ctx.manual.clone(), last_denied).await?;
        let run = Arc::new(Running {
            browser: tokio::sync::Mutex::new(browser),
            gate,
            proxy: Arc::new(proxy),
            ctx,
            needs_login: AtomicBool::new(false),
        });
        map.insert(id.to_string(), run.clone());
        drop(map);
        self.emit("systems://state", json!({ "system_id": id }));
        Ok(run)
    }

    /// AI 的一次工具呼叫。
    pub async fn call(self: &Arc<Self>, system: &str, tool: &str, args: Value) -> Result<Value, String> {
        let run = self.ensure_running(system).await?;
        let conn = self.connection(system)?;
        if !conn.enabled {
            self.close(system).await;
            return Err("這個系統連線已停用".into());
        }
        *run.ctx.conn.lock() = conn.clone(); // 模式、規則改了立刻生效
        let mut res = run.proxy.call(tool, args).await?;
        let text = res["content"].as_array().map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
        let logged_out = looks_logged_out(&conn, &text);
        if run.needs_login.swap(logged_out, Ordering::SeqCst) != logged_out {
            self.emit("systems://state", json!({ "system_id": system, "needs_login": logged_out }));
        }
        if logged_out {
            if let Some(a) = res["content"].as_array_mut() {
                a.push(json!({ "type": "text", "text": "注意：頁面看起來是登入頁，登入可能已失效。請使用者在瀏覽器視窗重新登入，再告訴你繼續；不要嘗試自己填入帳號密碼。" }));
            }
        }
        Ok(res)
    }

    /// 停止：拒絕這個系統所有等待中的核准（瀏覽器保持開著）。
    pub fn stop(&self, id: &str) -> usize {
        let n = self.approvals.reject_all(id);
        self.emit("systems://state", json!({ "system_id": id }));
        n
    }

    pub async fn set_manual(&self, id: &str, manual: bool) -> Result<(), String> {
        let map = self.running.lock().await;
        let run = map.get(id).ok_or("這個系統目前沒有開啟")?;
        run.ctx.manual.store(manual, Ordering::SeqCst);
        drop(map);
        self.emit("systems://state", json!({ "system_id": id, "manual": manual }));
        Ok(())
    }

    /// 關閉這個系統的瀏覽器與代理（停用時用）。
    pub async fn close(&self, id: &str) {
        self.approvals.reject_all(id);
        if let Some(r) = self.running.lock().await.remove(id) {
            r.proxy.close().await;
            r.gate.abort();
            r.browser.lock().await.close().await;
        }
        self.emit("systems://state", json!({ "system_id": id }));
    }

    pub async fn status(&self, id: &str) -> SystemStatus {
        let map = self.running.lock().await;
        let pending = self.approvals.list().into_iter().filter(|p| p.pending.system_id == id).count();
        match map.get(id) {
            Some(r) => SystemStatus {
                running: true,
                manual: r.ctx.manual.load(Ordering::SeqCst),
                pending,
                needs_login: r.needs_login.load(Ordering::SeqCst),
            },
            None => SystemStatus { running: false, manual: false, pending, needs_login: false },
        }
    }

    pub fn pending(&self) -> Vec<PendingView> {
        self.approvals.list()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems::types::Mode;

    fn conn() -> SystemConnection {
        SystemConnection {
            id: "s".into(),
            name: "S".into(),
            start_url: "https://app.example.com/".into(),
            allowed_hosts: vec!["example.com".into()],
            mode: Mode::ReadOnly,
            rules: vec![],
            enabled: true,
        }
    }

    #[test]
    fn a_login_page_or_foreign_host_means_logged_out() {
        let c = conn();
        assert!(looks_logged_out(&c, "### Page\n- Page URL: https://app.example.com/login?next=/\n"));
        assert!(looks_logged_out(&c, "- Page URL: https://sso.idp.test/auth\n"), "導去別的網域（IdP）");
        assert!(!looks_logged_out(&c, "- Page URL: https://app.example.com/orders\n"));
        assert!(!looks_logged_out(&c, "沒有網址的結果"));
    }

    /// 需要真的 Chrome 與網路：`cargo test -p app --lib -- --ignored --test-threads=1 systems::state`。
    #[tokio::test]
    #[ignore]
    async fn end_to_end_call_refusal_takeover_and_disable() {
        use crate::systems::testsite::{conn, fake_site};
        use std::sync::atomic::Ordering;

        let (port, posts) = fake_site().await;
        let dir = tempfile::tempdir().unwrap();
        let cfg = Arc::new(ConfigStore::new_at(dir.path().join("config.json")));
        let c = conn(port, Mode::ReadOnly);
        cfg.update(|a| a.system_connections.push(c.clone())).unwrap();
        let st = Arc::new(SystemsState::new(cfg.clone()));
        let text = |v: &Value| v["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");

        let url = format!("http://127.0.0.1:{port}/");
        st.call("t", "sys_navigate", json!({"system":"t","url":url,"intent":"讀：開首頁"})).await.unwrap();
        let snap = st.call("t", "sys_snapshot", json!({"system":"t"})).await.unwrap();
        let t = text(&snap);
        let r = t.split("button \"Save\" [ref=").nth(1).and_then(|s| s.split(']').next()).expect(&t).to_string();
        let click = st.call("t", "sys_click", json!({"system":"t","target":r,"intent":"寫：按儲存"})).await.unwrap();
        assert!(text(&click).contains("被閘門拒絕"));
        assert_eq!(posts.load(Ordering::SeqCst), 0);

        // 接手期間 AI 的呼叫被擋下。
        st.set_manual("t", true).await.unwrap();
        let during = st.call("t", "sys_snapshot", json!({"system":"t"})).await.unwrap();
        assert!(text(&during).contains("接手"), "{}", text(&during));
        st.set_manual("t", false).await.unwrap();
        assert!(!text(&st.call("t", "sys_snapshot", json!({"system":"t"})).await.unwrap()).contains("接手操作中"));

        // 停用：關掉瀏覽器，之後的呼叫被拒絕。
        let mut off = c.clone();
        off.enabled = false;
        cfg.update(|a| a.system_connections[0] = off.clone()).unwrap();
        st.close("t").await;
        assert!(!st.status("t").await.running);
        assert!(st.call("t", "sys_snapshot", json!({"system":"t"})).await.is_err());
    }
}
