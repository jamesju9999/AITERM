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
use super::recipes::{self, Param, Step};
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
    /// 最近一次畫面快照的文字（把元素參照換成定位器用）。
    last_snapshot: Mutex<String>,
    /// 成功的動作類呼叫，給 `sys_recipe_save` 存成配方。上限 200 步。
    journal: Mutex<Vec<Step>>,
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

    /// 用指定的資料庫（測試用記憶體資料庫，免得測試把記錄寫進使用者真正的 systems.db）。
    pub fn with_pool(config: Arc<ConfigStore>, pool: SqlitePool) -> Self {
        let s = Self::new(config);
        let _ = s.audit.set(Some(pool));
        s
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
        proxy.warm_up().await;
        let run = Arc::new(Running {
            browser: tokio::sync::Mutex::new(browser),
            gate,
            proxy: Arc::new(proxy),
            ctx,
            needs_login: AtomicBool::new(false),
            last_snapshot: Mutex::new(String::new()),
            journal: Mutex::new(Vec::new()),
        });
        map.insert(id.to_string(), run.clone());
        drop(map);
        self.emit("systems://state", json!({ "system_id": id }));
        Ok(run)
    }

    /// AI 的一次工具呼叫。
    pub async fn call(self: &Arc<Self>, system: &str, tool: &str, args: Value) -> Result<Value, String> {
        self.call_inner(system, tool, args, true).await
    }

    /// `record`：成功的動作要不要進操作日誌（重播配方時不記，免得存下重播本身）。
    async fn call_inner(self: &Arc<Self>, system: &str, tool: &str, args: Value, record: bool) -> Result<Value, String> {
        let run = self.ensure_running(system).await?;
        let conn = self.connection(system)?;
        if !conn.enabled {
            self.close(system).await;
            return Err("這個系統連線已停用".into());
        }
        *run.ctx.conn.lock() = conn.clone(); // 模式、規則改了立刻生效
        // 日誌用的步驟：把元素參照換成定位器（要在呼叫前用「呼叫當下看到的快照」換）。
        let journal_step = (record && recipes::is_replayable(tool)).then(|| {
            let entries = recipes::parse_snapshot(&run.last_snapshot.lock());
            let mut a = recipes::to_locators(&args, &entries);
            let intent = a["intent"].as_str().unwrap_or("").to_string();
            if let Some(o) = a.as_object_mut() {
                o.remove("system");
                o.remove("intent");
            }
            Step { tool: tool.to_string(), args: a, intent }
        });
        let mut res = run.proxy.call(tool, args).await?;
        let text = res["content"].as_array().map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
        if text.contains("[ref=") {
            *run.last_snapshot.lock() = text.clone();
        }
        if let Some(step) = journal_step {
            if res["isError"] != true {
                let mut j = run.journal.lock();
                j.push(step);
                if j.len() > 200 {
                    j.remove(0);
                }
            }
        }
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

    /// 把操作日誌（可只取最後 `last_n` 步）存成配方，並清空日誌。
    pub async fn recipe_save(
        &self,
        system: &str,
        name: &str,
        description: &str,
        params: Vec<Param>,
        last_n: Option<usize>,
    ) -> Result<String, String> {
        self.connection(system)?;
        if name.trim().is_empty() {
            return Err("配方需要名稱".into());
        }
        let run = self.running.lock().await.get(system).cloned().ok_or("這個系統目前沒有開啟，沒有可存的操作")?;
        let mut steps: Vec<Step> = run.journal.lock().clone();
        if let Some(n) = last_n {
            let skip = steps.len().saturating_sub(n);
            steps.drain(..skip);
        }
        if steps.is_empty() {
            return Err("操作日誌是空的：先成功完成一串操作，再存成配方".into());
        }
        for s in &mut steps {
            s.args = recipes::parameterize(&s.args, &params);
        }
        let pool = self.audit_pool().await.ok_or("審計資料庫不可用，無法儲存配方")?;
        recipes::save(&pool, system, name.trim(), description, &params, &steps).await.map_err(|e| e.to_string())?;
        run.journal.lock().clear();
        Ok(format!("已存成配方「{}」，共 {} 步、{} 個參數。", name.trim(), steps.len(), params.len()))
    }

    pub async fn recipe_list(&self, system: &str) -> Result<Vec<recipes::Recipe>, String> {
        self.connection(system)?;
        let pool = self.audit_pool().await.ok_or("審計資料庫不可用")?;
        recipes::list(&pool, system).await.map_err(|e| e.to_string())
    }

    pub async fn recipe_delete(&self, id: i64) -> Result<bool, String> {
        let pool = self.audit_pool().await.ok_or("審計資料庫不可用")?;
        recipes::delete(&pool, id).await.map_err(|e| e.to_string())
    }

    /// 重播配方。每一步都走一般的呼叫流程（intent、閘門、核准、審計都照常）；
    /// 任何一步失敗就停下並回報。
    pub async fn recipe_run(
        self: &Arc<Self>,
        system: &str,
        name: &str,
        values: std::collections::HashMap<String, String>,
    ) -> Result<String, String> {
        self.connection(system)?;
        let pool = self.audit_pool().await.ok_or("審計資料庫不可用")?;
        let recipe = recipes::get(&pool, system, name)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("找不到配方：{name}"))?;
        let missing = recipes::missing_params(&recipe.params, &values);
        if !missing.is_empty() {
            return Err(format!("缺少參數：{}", missing.join("、")));
        }
        let total = recipe.steps.len();
        let text_of = |v: &Value| -> String {
            v["content"].as_array().map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
        };
        for (i, step) in recipe.steps.iter().enumerate() {
            let n = i + 1;
            let fail = |why: String| format!("配方「{name}」在第 {n}/{total} 步停下：{why}（步驟：{}）", step.intent);
            let prepared: Result<Value, String> = async {
                let args = recipes::apply_params(&step.args, &values)?;
                if step.tool == "sys_navigate" {
                    return Ok(args);
                }
                let snap = self.call_inner(system, "sys_snapshot", json!({ "system": system }), false).await?;
                let entries = recipes::parse_snapshot(&text_of(&snap));
                recipes::resolve_locators(&args, &entries)
            }
            .await;
            let mut args = match prepared {
                Ok(a) => a,
                Err(e) => {
                    let _ = recipes::record_run(&pool, recipe.id, false).await;
                    return Err(fail(e));
                }
            };
            if let Some(o) = args.as_object_mut() {
                o.insert("system".into(), json!(system));
                o.insert("intent".into(), json!(step.intent));
            }
            match self.call_inner(system, &step.tool, args, false).await {
                Ok(r) if r["isError"] != true => {}
                Ok(r) => {
                    let _ = recipes::record_run(&pool, recipe.id, false).await;
                    return Err(fail(text_of(&r)));
                }
                Err(e) => {
                    let _ = recipes::record_run(&pool, recipe.id, false).await;
                    return Err(fail(e));
                }
            }
        }
        let _ = recipes::record_run(&pool, recipe.id, true).await;
        Ok(format!("配方「{name}」已完成，共 {total} 步。"))
    }

    /// 停止：拒絕這個系統所有等待中的核准，並讓 AI 的工具呼叫暫停（瀏覽器保持開著），
    /// 直到使用者按「交還 AI」。回傳拒絕了幾筆。
    pub async fn stop(&self, id: &str) -> usize {
        let n = self.approvals.reject_all(id);
        if let Some(r) = self.running.lock().await.get(id) {
            r.ctx.manual.store(true, Ordering::SeqCst);
        }
        self.emit("systems://state", json!({ "system_id": id, "manual": true }));
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
        let st = Arc::new(SystemsState::with_pool(cfg.clone(), crate::systems::audit::open_in_memory().await));
        let text = |v: &Value| v["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");

        let url = format!("http://127.0.0.1:{port}/");
        st.call("t", "sys_navigate", json!({"system":"t","url":url,"intent":"讀：開首頁"})).await.unwrap();
        let snap = st.call("t", "sys_snapshot", json!({"system":"t"})).await.unwrap();
        let t = text(&snap);
        let r = t.split("button \"Save\" [ref=").nth(1).and_then(|s| s.split(']').next()).expect(&t).to_string();
        let click = st.call("t", "sys_click", json!({"system":"t","target":r,"intent":"寫：按儲存"})).await.unwrap();
        assert!(text(&click).contains("被閘門拒絕"));
        assert_eq!(posts.load(Ordering::SeqCst), 0);

        // 停止：AI 的呼叫被擋下，直到交還。
        st.stop("t").await;
        assert!(st.status("t").await.manual);
        let stopped = st.call("t", "sys_snapshot", json!({"system":"t"})).await.unwrap();
        assert!(text(&stopped).contains("接手"), "{}", text(&stopped));
        st.set_manual("t", false).await.unwrap();

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

    /// 配方：AI 成功填單送出（逐次核准）→ 存成配方 → 換參數重播 → 伺服器收到新值；
    /// 改成只讀後重播被拒、伺服器沒再收到。需要真的 Chrome 與網路：
    /// `cargo test -p app --lib -- --ignored --test-threads=1 systems::state`
    #[tokio::test]
    #[ignore]
    async fn a_recipe_is_recorded_replayed_with_new_params_and_still_obeys_the_gate() {
        use crate::systems::testsite::{conn, fake_site_with_bodies};
        use std::collections::HashMap;
        use std::sync::atomic::Ordering;

        let (port, posts, bodies) = fake_site_with_bodies().await;
        let dir = tempfile::tempdir().unwrap();
        let cfg = Arc::new(ConfigStore::new_at(dir.path().join("config.json")));
        cfg.update(|a| a.system_connections.push(conn(port, Mode::ApproveEach))).unwrap();
        let st = Arc::new(SystemsState::with_pool(cfg.clone(), crate::systems::audit::open_in_memory().await));
        // 自動核准：每 200ms 看一次有沒有等待中的。
        let approver = st.clone();
        let auto = tokio::spawn(async move {
            loop {
                for p in approver.approvals.list() {
                    approver.approvals.decide(&p.id, true);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
        let text = |v: &Value| v["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");

        let url = format!("http://127.0.0.1:{port}/");
        st.call("t", "sys_navigate", json!({"system":"t","url":url,"intent":"讀：開表單"})).await.unwrap();
        let snap = st.call("t", "sys_snapshot", json!({"system":"t"})).await.unwrap();
        let entries = recipes::parse_snapshot(&text(&snap));
        let input = entries.iter().find(|e| e.role == "textbox").unwrap().reference.clone();
        let button = entries.iter().find(|e| e.role == "button").unwrap().reference.clone();
        st.call("t", "sys_type", json!({"system":"t","target":input,"text":"A-102","intent":"寫：填單號"})).await.unwrap();
        let click = st.call("t", "sys_click", json!({"system":"t","target":button,"intent":"寫：送出表單"})).await.unwrap();
        assert!(click["isError"] != true, "{}", text(&click));
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        assert!(bodies.lock()[0].contains("a=A-102"), "{:?}", bodies.lock());

        let msg = st
            .recipe_save("t", "送單", "填單號並送出", vec![Param { name: "order_no".into(), example: "A-102".into() }], None)
            .await
            .unwrap();
        assert!(msg.contains("3 步"), "{msg}");
        assert!(st.recipe_save("t", "空", "", vec![], None).await.is_err(), "日誌清空後不能再存");

        // 重播：換參數，伺服器收到新值。
        let vals: HashMap<String, String> = [("order_no".to_string(), "B-7".to_string())].into();
        let out = st.recipe_run("t", "送單", vals.clone()).await.unwrap();
        assert!(out.contains("已完成"), "{out}");
        assert_eq!(posts.load(Ordering::SeqCst), 2);
        assert!(bodies.lock()[1].contains("a=B-7"), "{:?}", bodies.lock());
        assert!(st.recipe_run("t", "送單", HashMap::new()).await.unwrap_err().contains("缺少參數"));

        // 改成只讀：重播在寫入那一步被閘門拒絕，伺服器沒再收到。
        let mut ro = conn(port, Mode::ReadOnly);
        ro.id = "t".into();
        cfg.update(|a| a.system_connections[0] = ro.clone()).unwrap();
        let err = st.recipe_run("t", "送單", vals).await.unwrap_err();
        assert!(err.contains("停下") && err.contains("被閘門拒絕"), "{err}");
        assert_eq!(posts.load(Ordering::SeqCst), 2);

        let list = st.recipe_list("t").await.unwrap();
        assert_eq!((list[0].ok_count, list[0].fail_count), (1, 1));

        auto.abort();
        st.close("t").await;
    }
}
