//! 桌面 App 連線的執行核心：每個 `sys_app_*` 工具呼叫都在這裡經過
//! 「停用／暫停／人機衝突檢查 → 取動作對象 → 動作層閘門 → 必要時等核准 → 交給電腦操作 → 審計」。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::gate::{self, Action};
use super::presence::{self, InputMonitor};
use super::tools::{self, TOOL_LOGIN, TOOL_NEEDS_LOGIN};
use crate::computer::{self, with_pre_confirmed, ComputerService};
use crate::systems::approvals::{Approvals, Outcome, Pending, APPROVAL_TIMEOUT};
use crate::systems::audit::{self, Entry, Verdict as Audit};
use crate::systems::login::Creds;
use crate::systems::state::Emitter;
use crate::systems::types::{Decision, DenyReason, SystemConnection};

/// 登入失敗時別連續重試（帳密錯會把帳號鎖住）。
const LOGIN_COOLDOWN: Duration = Duration::from_secs(60);
const PAUSED_MSG: &str = "已暫停：使用者正在操作這台電腦或按了停止。不要再呼叫工具；\
用一句話回報「被使用者暫停」後結束。使用者按「繼續」之後才會恢復。";

#[derive(Default)]
struct Session {
    paused: bool,
    needs_login: bool,
    /// 上一次自己送出輸入（點擊、打字、按鍵）完成的時間，用來分辨「別人動了滑鼠鍵盤」。
    last_own_input: Option<Instant>,
    last_login: Option<Instant>,
}

pub struct DesktopRunner {
    computer: Arc<ComputerService>,
    monitor: Arc<dyn InputMonitor>,
    approvals: Arc<Approvals>,
    emit: Emitter,
    creds: Arc<dyn Fn(&str) -> Option<Creds> + Send + Sync>,
    sessions: Mutex<HashMap<String, Session>>,
    approval_timeout: Duration,
    /// 找不到 App 而自動啟動後，等它開起來的時間。
    launch_wait: Duration,
}

fn text(t: impl Into<String>, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }], "isError": is_error })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl DesktopRunner {
    pub fn new(
        computer: Arc<ComputerService>,
        monitor: Arc<dyn InputMonitor>,
        approvals: Arc<Approvals>,
        emit: Emitter,
        creds: Arc<dyn Fn(&str) -> Option<Creds> + Send + Sync>,
    ) -> Self {
        Self {
            computer,
            monitor,
            approvals,
            emit,
            creds,
            sessions: Mutex::new(HashMap::new()),
            approval_timeout: APPROVAL_TIMEOUT,
            launch_wait: Duration::from_secs(2),
        }
    }

    #[cfg(test)]
    pub fn with_timings(mut self, approval_timeout: Duration, launch_wait: Duration) -> Self {
        self.approval_timeout = approval_timeout;
        self.launch_wait = launch_wait;
        self
    }

    #[cfg(test)]
    pub fn backdate_own_input(&self, id: &str, secs: u64) {
        self.sessions.lock().entry(id.to_string()).or_default().last_own_input = Some(Instant::now() - Duration::from_secs(secs));
    }

    pub fn monitor(&self) -> &Arc<dyn InputMonitor> {
        &self.monitor
    }

    /// `(已暫停, 需要使用者登入)`。
    pub fn status(&self, id: &str) -> (bool, bool) {
        self.sessions.lock().get(id).map(|s| (s.paused, s.needs_login)).unwrap_or((false, false))
    }

    fn emit_state(&self, id: &str) {
        let (paused, needs_login) = self.status(id);
        (self.emit)("systems://state", json!({ "system_id": id, "manual": paused, "needs_login": needs_login }));
    }

    /// 停止或使用者接手：拒絕等待中的核准，之後的工具呼叫都回「已暫停」。
    pub fn pause(&self, id: &str) -> usize {
        let n = self.approvals.reject_all(id);
        self.sessions.lock().entry(id.to_string()).or_default().paused = true;
        self.emit_state(id);
        n
    }

    /// 使用者按「繼續」：解除暫停與登入旗標，並重新計算「自己最後一次輸入」。
    pub fn resume(&self, id: &str) {
        {
            let mut m = self.sessions.lock();
            let s = m.entry(id.to_string()).or_default();
            s.paused = false;
            s.needs_login = false;
            s.last_own_input = None;
        }
        self.emit_state(id);
    }

    pub async fn open_app(&self, conn: &SystemConnection) -> Result<(), String> {
        self.computer.authorize_app_for_system(&[conn.app.clone()]);
        self.computer.open_app(&conn.app).await.map(|_| ())
    }

    /// 連線停用或移除：收回對這個 App 的預先授權。
    pub fn revoke(&self, app: &str) {
        self.computer.revoke_app_for_system(&[app.to_string()]);
    }

    /// 停用或移除連線時清掉狀態。
    pub fn close(&self, id: &str) {
        self.approvals.reject_all(id);
        self.sessions.lock().remove(id);
        self.emit_state(id);
    }

    /// 動作之前：有人動過滑鼠鍵盤就暫停。回傳是否已（或剛剛）暫停。
    fn check_paused_or_interfered(&self, id: &str) -> bool {
        let mut m = self.sessions.lock();
        let s = m.entry(id.to_string()).or_default();
        if s.paused {
            return true;
        }
        if let (Some(last), Some(idle)) = (s.last_own_input, self.monitor.idle()) {
            if presence::user_interfered(idle, last.elapsed(), presence::TOLERANCE) {
                s.paused = true;
                drop(m);
                self.emit_state(id);
                return true;
            }
        }
        false
    }

    fn mark_own_input(&self, id: &str) {
        self.sessions.lock().entry(id.to_string()).or_default().last_own_input = Some(Instant::now());
    }

    async fn record(&self, pool: Option<&SqlitePool>, mut e: Entry, result: Option<&str>) {
        if let Some(pool) = pool {
            e.result = result.map(|r| r.chars().take(500).collect());
            if let Err(err) = audit::record(pool, &e).await {
                log::warn!("桌面系統審計寫入失敗：{err}");
            }
        }
    }

    /// 動作對象的描述（元件標題、要點的文字、按鍵），給閘門判定與核准卡顯示。
    async fn describe(&self, app: &str, short: &str, args: &Value) -> (&'static str, String) {
        let element = |v: &Value| v.as_str().map(str::to_string);
        match short {
            "click" => {
                let label = if let Some(id) = element(&args["element"]) {
                    self.computer.element_label(app, &id).await.map(|(_, t)| t)
                } else if let (Some(x), Some(y)) = (args["x"].as_f64(), args["y"].as_f64()) {
                    self.computer.label_at(x, y).await.map(|(_, t)| t)
                } else {
                    None
                };
                ("click", label.unwrap_or_default())
            }
            "click_text" => ("click", args["text"].as_str().unwrap_or("").to_string()),
            "type" => {
                let label = match element(&args["element"]) {
                    Some(id) => self.computer.element_label(app, &id).await.map(|(_, t)| t),
                    None => None,
                };
                ("type", label.unwrap_or_default())
            }
            "key" => ("key", args["key"].as_str().unwrap_or("").to_string()),
            "scroll" => ("scroll", String::new()),
            _ => ("read", String::new()),
        }
    }

    /// 閘門判定＋必要時等使用者核准。放行回 `Ok(is_write)`；拒絕回要交給 AI 的錯誤結果。
    async fn authorize(
        &self,
        conn: &SystemConnection,
        pool: Option<&SqlitePool>,
        tool: &str,
        intent: &str,
        kind: &str,
        label: &str,
    ) -> Result<bool, Value> {
        let action = Action { kind, label };
        let v = gate::decide(conn, &action, gate::declares_write(intent));
        let target = format!("{}：{}", conn.app, if label.is_empty() { kind } else { label });
        let entry = |verdict: Audit, source: &str| Entry::new(&conn.id, tool, intent, Some(kind), Some(&target), verdict, source);
        let log_it = kind != "read";
        match v.decision {
            Decision::Allow => {
                if log_it {
                    self.record(pool, entry(Audit::Allow, "rule"), None).await;
                }
                Ok(v.is_write)
            }
            Decision::Deny(reason) => {
                self.record(pool, entry(Audit::Deny, "rule"), None).await;
                let why = match reason {
                    DenyReason::Disabled => "這個系統連線已停用",
                    DenyReason::ReadOnly => "這個系統是只讀，不允許寫入",
                    DenyReason::HostNotAllowed => "目標不在允許範圍",
                };
                Err(text(
                    format!("被閘門拒絕：{why}（動作：{kind}「{label}」）。不要重試；停下來，回報需要開放什麼權限。"),
                    true,
                ))
            }
            Decision::Ask => {
                let pending = Pending {
                    system_id: conn.id.clone(),
                    intent: intent.to_string(),
                    method: kind.to_string(),
                    url: target.clone(),
                    declared_read: v.declared_read,
                    ts_ms: now_ms(),
                };
                let (id, rx) = self.approvals.open(pending.clone());
                (self.emit)("systems://need-approval", json!({ "id": id, "pending": pending }));
                let outcome = Approvals::wait(rx, self.approval_timeout).await;
                if outcome == Outcome::TimedOut {
                    self.approvals.expire(&id);
                }
                (self.emit)("systems://resolved", json!({ "id": id }));
                match outcome {
                    Outcome::Approved => {
                        self.record(pool, entry(Audit::Approved, "user"), None).await;
                        Ok(true)
                    }
                    Outcome::Rejected => {
                        self.record(pool, entry(Audit::Rejected, "user"), None).await;
                        Err(text(format!("使用者拒絕了這個動作（{kind}「{label}」）。被閘門拒絕，不要重試；停下來回報。"), true))
                    }
                    Outcome::TimedOut => {
                        self.record(pool, entry(Audit::Timeout, "rule"), None).await;
                        Err(text(format!("等使用者核准逾時（{kind}「{label}」）。被閘門拒絕，不要重試；停下來回報。"), true))
                    }
                }
            }
        }
    }

    /// 呼叫電腦操作工具；App 沒開且連線允許時先啟動再重試一次。
    async fn run_computer(&self, conn: &SystemConnection, cname: &str, args: &Value, pre_confirmed: bool) -> (String, bool) {
        let mut a = args.clone();
        if let Some(o) = a.as_object_mut() {
            o.remove("system");
            o.remove("intent");
            o.insert("app".into(), json!(conn.app));
        }
        let exec = |a: Value| async move {
            if pre_confirmed {
                with_pre_confirmed(computer::tools::call(&self.computer, cname, a)).await
            } else {
                computer::tools::call(&self.computer, cname, a).await
            }
        };
        let mut res = exec(a.clone()).await;
        if res.is_error && res.content.contains("找不到 App") && conn.launch_app {
            match self.computer.open_app(&conn.app).await {
                Ok(_) => {
                    if !self.launch_wait.is_zero() {
                        tokio::time::sleep(self.launch_wait).await;
                    }
                    res = exec(a).await;
                }
                Err(e) => return (format!("找不到 App「{}」，而且自動啟動失敗：{e}", conn.app), true),
            }
        }
        (res.content, res.is_error)
    }

    pub async fn call(&self, conn: &SystemConnection, pool: Option<&SqlitePool>, tool: &str, args: Value) -> Result<Value, String> {
        if !conn.enabled {
            self.close(&conn.id);
            return Err("這個系統連線已停用".into());
        }
        if conn.app.trim().is_empty() {
            return Err("這個桌面系統沒有設定要綁定的 App".into());
        }
        let intent = args["intent"].as_str().unwrap_or("").trim().to_string();
        if intent.is_empty() {
            return Ok(text("缺少 intent 參數：請以「讀：」或「寫：」開頭說明這個動作要做什麼。", true));
        }
        // 使用者建立連線＝授權這個 App（每次呼叫都登記，設定改了立刻生效）。
        self.computer.authorize_app_for_system(&[conn.app.clone()]);
        if self.check_paused_or_interfered(&conn.id) {
            return Ok(text(PAUSED_MSG, true));
        }
        match tool {
            TOOL_NEEDS_LOGIN => {
                self.sessions.lock().entry(conn.id.clone()).or_default().needs_login = true;
                self.emit_state(&conn.id);
                let reason = args["reason"].as_str().unwrap_or("");
                self.record(pool, Entry::new(&conn.id, tool, &intent, Some("login"), Some(&conn.app), Audit::Allow, "rule"), Some(reason)).await;
                Ok(text("已通知使用者需要登入。請停下來，用一兩句話回報目前卡在登入；不要嘗試自己輸入帳密。", false))
            }
            TOOL_LOGIN => self.login(conn, pool, &intent, &args).await,
            _ => {
                let Some(cname) = tools::computer_name(tool) else {
                    return Ok(text(format!("這個桌面系統沒有 {tool} 這個工具"), true));
                };
                let short = tool.trim_start_matches("sys_app_");
                let (kind, label) = self.describe(&conn.app, short, &args).await;
                let write = match self.authorize(conn, pool, tool, &intent, kind, &label).await {
                    Ok(w) => w,
                    Err(e) => return Ok(e),
                };
                let (content, is_err) = self.run_computer(conn, &cname, &args, write).await;
                if matches!(kind, "click" | "type" | "key" | "scroll") {
                    self.mark_own_input(&conn.id);
                }
                Ok(text(content, is_err))
            }
        }
    }

    async fn login(&self, conn: &SystemConnection, pool: Option<&SqlitePool>, intent: &str, args: &Value) -> Result<Value, String> {
        let (user_el, pass_el) = match (args["username_element"].as_str(), args["password_element"].as_str()) {
            (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => (u.to_string(), p.to_string()),
            _ => return Ok(text("缺少 username_element 或 password_element（元件 id 來自 sys_app_snapshot／sys_app_find）。", true)),
        };
        let Some(creds) = (self.creds)(&conn.id) else {
            self.sessions.lock().entry(conn.id.clone()).or_default().needs_login = true;
            self.emit_state(&conn.id);
            return Ok(text("這個系統沒有存帳密。已通知使用者需要登入；請停下來回報，不要嘗試自己輸入帳密。", true));
        };
        let recently = self.sessions.lock().get(&conn.id).and_then(|s| s.last_login).is_some_and(|t| t.elapsed() < LOGIN_COOLDOWN);
        if recently {
            return Ok(text("剛剛才登入過，帳密可能不對。請停下來，改用 sys_app_report_login_needed 回報，不要再試。", true));
        }
        if let Err(e) = self.authorize(conn, pool, TOOL_LOGIN, intent, "login", "登入").await {
            return Ok(e);
        }
        self.sessions.lock().entry(conn.id.clone()).or_default().last_login = Some(Instant::now());
        let steps = async {
            self.computer.type_text(&conn.app, &creds.username, Some(&user_el), true).await?;
            self.computer.type_secret(&conn.app, &pass_el, &creds.password).await?;
            if args["submit"].as_str() == Some("return") {
                with_pre_confirmed(self.computer.key(&conn.app, "return")).await?;
            }
            Ok::<(), String>(())
        }
        .await;
        self.mark_own_input(&conn.id);
        match steps {
            Ok(()) => {
                self.sessions.lock().entry(conn.id.clone()).or_default().needs_login = false;
                self.emit_state(&conn.id);
                Ok(text("已填入帳號與密碼（內容不回傳）。請用 sys_app_snapshot 確認是否登入成功；若仍在登入畫面，用 sys_app_report_login_needed 回報。", false))
            }
            // 錯誤訊息只來自電腦操作層，不含密碼；保險起見再遮一次。
            Err(e) => Ok(text(format!("登入沒有完成：{}", e.replace(&creds.password, "***")), true)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::approval::Decision as UserDecision;
    use crate::computer::tests::{test_service, FakeApprover, FakeBackend};
    use crate::systems::desktop::presence::FakeMonitor;
    use crate::systems::types::{Mode, SystemKind};

    struct Rig {
        runner: Arc<DesktopRunner>,
        backend: Arc<FakeBackend>,
        approver: Arc<FakeApprover>,
        monitor: Arc<FakeMonitor>,
        approvals: Arc<Approvals>,
        pool: SqlitePool,
        creds: Arc<Mutex<Option<Creds>>>,
        events: Arc<Mutex<Vec<String>>>,
        _dir: tempfile::TempDir,
    }

    async fn rig() -> Rig {
        let (computer, backend, approver, dir) = test_service(&[]);
        let monitor = FakeMonitor::new(Some(Duration::from_secs(3600)), Some(false));
        let approvals = Arc::new(Approvals::new());
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let ev = events.clone();
        let emit: Emitter = Arc::new(move |name, payload| ev.lock().push(format!("{name} {payload}")));
        let creds: Arc<Mutex<Option<Creds>>> = Arc::new(Mutex::new(None));
        let c2 = creds.clone();
        let runner = Arc::new(
            DesktopRunner::new(computer, monitor.clone(), approvals.clone(), emit, Arc::new(move |_| c2.lock().clone()))
                .with_timings(Duration::from_millis(300), Duration::ZERO),
        );
        Rig { runner, backend, approver, monitor, approvals, pool: audit::open_in_memory().await, creds, events, _dir: dir }
    }

    fn conn(mode: Mode) -> SystemConnection {
        SystemConnection { id: "erp".into(), name: "ERP".into(), kind: SystemKind::Desktop, app: "Notes".into(), mode, ..Default::default() }
    }

    async fn call(r: &Rig, c: &SystemConnection, tool: &str, args: Value) -> Value {
        r.runner.call(c, Some(&r.pool), tool, args).await.unwrap()
    }

    fn out(v: &Value) -> String {
        v["content"][0]["text"].as_str().unwrap_or("").to_string()
    }

    fn is_err(v: &Value) -> bool {
        v["isError"] == true
    }

    async fn verdicts(r: &Rig) -> Vec<String> {
        audit::list(&r.pool, "erp", 50).await.unwrap().into_iter().map(|x| x.verdict).collect()
    }

    #[tokio::test]
    async fn the_bound_app_is_forced_whatever_the_ai_passes() {
        let r = rig().await;
        let v = call(&r, &conn(Mode::ReadOnly), "sys_app_snapshot", json!({ "system": "erp", "intent": "讀：看畫面", "app": "Photoshop" })).await;
        assert!(!is_err(&v), "{}", out(&v));
        assert!(out(&v).contains("Save") && out(&v).contains("Delete"));
        assert!(r.approver.asked().is_empty(), "連線已授權綁定的 App，不彈首次使用對話框");
    }

    #[tokio::test]
    async fn read_only_allows_plain_reads_and_denies_a_dangerous_click_even_if_declared_read() {
        let r = rig().await;
        let c = conn(Mode::ReadOnly);
        call(&r, &c, "sys_app_snapshot", json!({ "system": "erp", "intent": "讀：看" })).await;
        let ok = call(&r, &c, "sys_app_click", json!({ "system": "erp", "intent": "讀：點儲存", "element": "0.0" })).await;
        assert!(!is_err(&ok), "{}", out(&ok));
        let denied = call(&r, &c, "sys_app_click", json!({ "system": "erp", "intent": "讀：看看", "element": "0.1" })).await;
        assert!(is_err(&denied) && out(&denied).contains("被閘門拒絕"), "{}", out(&denied));
        assert!(!r.backend.calls().iter().any(|c| c == "press 0.1"), "Delete 一次都沒被按");
        assert!(verdicts(&r).await.contains(&"deny".to_string()));
    }

    #[tokio::test]
    async fn read_only_denies_enter_and_a_declared_write() {
        let r = rig().await;
        let c = conn(Mode::ReadOnly);
        for args in [json!({ "system": "erp", "intent": "讀：送出", "key": "return" }), json!({ "system": "erp", "intent": "寫：改備註", "key": "a" })] {
            let v = call(&r, &c, "sys_app_key", args).await;
            assert!(is_err(&v) && out(&v).contains("被閘門拒絕"), "{}", out(&v));
        }
        assert!(!r.backend.calls().iter().any(|c| c.starts_with("key")));
    }

    #[tokio::test]
    async fn approve_each_waits_for_the_user_then_runs_without_a_second_dialog() {
        let r = rig().await;
        let c = conn(Mode::ApproveEach);
        call(&r, &c, "sys_app_snapshot", json!({ "system": "erp", "intent": "讀：看" })).await;
        let (runner, c2) = (r.runner.clone(), c.clone());
        let pool = r.pool.clone();
        let h = tokio::spawn(async move {
            runner.call(&c2, Some(&pool), "sys_app_click", json!({ "system": "erp", "intent": "寫：刪掉這筆", "element": "0.1" })).await.unwrap()
        });
        let id = loop {
            if let Some(p) = r.approvals.list().first() {
                assert_eq!(p.pending.method, "click");
                assert!(p.pending.url.contains("Notes") && p.pending.url.contains("Delete"), "{}", p.pending.url);
                break p.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        assert!(r.approvals.decide(&id, true));
        let v = h.await.unwrap();
        assert!(!is_err(&v), "{}", out(&v));
        assert!(r.backend.calls().iter().any(|c| c == "press 0.1"));
        assert!(r.approver.asked().is_empty(), "核准過了，電腦操作層不再重複問破壞性確認");
        assert!(verdicts(&r).await.contains(&"approved".to_string()));
    }

    #[tokio::test]
    async fn a_rejected_or_unanswered_approval_does_nothing() {
        let r = rig().await;
        let c = conn(Mode::ApproveEach);
        call(&r, &c, "sys_app_snapshot", json!({ "system": "erp", "intent": "讀：看" })).await;
        // 逾時（沒人回應）
        let v = call(&r, &c, "sys_app_click", json!({ "system": "erp", "intent": "寫：刪", "element": "0.1" })).await;
        assert!(is_err(&v) && out(&v).contains("逾時"), "{}", out(&v));
        assert!(r.approvals.list().is_empty(), "逾時後核准卡要移除");
        assert!(!r.backend.calls().iter().any(|c| c == "press 0.1"));
        assert!(verdicts(&r).await.contains(&"timeout".to_string()));
    }

    #[tokio::test]
    async fn autonomous_runs_a_dangerous_click_without_asking_anyone() {
        let r = rig().await;
        let c = conn(Mode::Autonomous);
        call(&r, &c, "sys_app_snapshot", json!({ "system": "erp", "intent": "讀：看" })).await;
        let v = call(&r, &c, "sys_app_click", json!({ "system": "erp", "intent": "寫：刪", "element": "0.1" })).await;
        assert!(!is_err(&v), "{}", out(&v));
        assert!(r.approver.asked().is_empty() && r.approvals.list().is_empty());
    }

    #[tokio::test]
    async fn missing_intent_and_unknown_tools_are_reported_not_executed() {
        let r = rig().await;
        let c = conn(Mode::Autonomous);
        assert!(is_err(&call(&r, &c, "sys_app_snapshot", json!({ "system": "erp" })).await));
        let v = call(&r, &c, "sys_app_open_app", json!({ "system": "erp", "intent": "讀：開" })).await;
        assert!(is_err(&v) && out(&v).contains("沒有"), "{}", out(&v));
        assert!(!r.backend.calls().iter().any(|c| c.starts_with("open_app")));
    }

    #[tokio::test]
    async fn a_disabled_connection_is_refused() {
        let r = rig().await;
        let mut c = conn(Mode::Autonomous);
        c.enabled = false;
        assert!(r.runner.call(&c, Some(&r.pool), "sys_app_snapshot", json!({ "system": "erp", "intent": "讀：x" })).await.is_err());
    }

    async fn login_rig(mode: Mode) -> (Rig, SystemConnection) {
        let r = rig().await;
        let c = conn(mode);
        *r.creds.lock() = Some(Creds { username: "alice".into(), password: "hunter2".into() });
        r.backend.doc.lock().2 = true; // 焦點是密碼欄位
        call(&r, &c, "sys_app_snapshot", json!({ "system": "erp", "intent": "讀：看登入畫面" })).await;
        (r, c)
    }

    fn login_args() -> Value {
        json!({ "system": "erp", "intent": "寫：登入", "username_element": "0.0", "password_element": "0.1", "submit": "return" })
    }

    #[tokio::test]
    async fn login_fills_credentials_without_ever_returning_the_password() {
        let (r, c) = login_rig(Mode::Autonomous).await;
        let v = call(&r, &c, "sys_app_login", login_args()).await;
        assert!(!is_err(&v), "{}", out(&v));
        let calls = r.backend.calls();
        assert!(calls.iter().any(|c| c == "type alice") && calls.iter().any(|c| c == "type hunter2"), "{calls:?}");
        assert!(calls.iter().any(|c| c == "key return"));
        assert!(!out(&v).contains("hunter2") && !out(&v).contains("alice"));
        for e in audit::list(&r.pool, "erp", 50).await.unwrap() {
            let all = format!("{:?}", e);
            assert!(!all.contains("hunter2"), "審計不可含密碼：{all}");
        }
        assert!(r.events.lock().iter().all(|e| !e.contains("hunter2")));
    }

    #[tokio::test]
    async fn a_second_login_within_the_cooldown_is_refused() {
        let (r, c) = login_rig(Mode::Autonomous).await;
        call(&r, &c, "sys_app_login", login_args()).await;
        let before = r.backend.calls().len();
        let v = call(&r, &c, "sys_app_login", login_args()).await;
        assert!(is_err(&v) && out(&v).contains("剛剛才登入過"), "{}", out(&v));
        assert_eq!(r.backend.calls().len(), before, "冷卻期間什麼都不輸入");
    }

    #[tokio::test]
    async fn login_into_a_non_password_field_types_nothing_secret() {
        let (r, c) = login_rig(Mode::Autonomous).await;
        r.backend.doc.lock().2 = false;
        let v = call(&r, &c, "sys_app_login", login_args()).await;
        assert!(is_err(&v) && out(&v).contains("密碼欄位") && !out(&v).contains("hunter2"), "{}", out(&v));
        assert!(!r.backend.calls().iter().any(|c| c == "type hunter2"));
    }

    #[tokio::test]
    async fn login_is_a_write_so_read_only_refuses_it() {
        let (r, c) = login_rig(Mode::ReadOnly).await;
        let v = call(&r, &c, "sys_app_login", login_args()).await;
        assert!(is_err(&v) && out(&v).contains("被閘門拒絕"), "{}", out(&v));
        assert!(!r.backend.calls().iter().any(|c| c.starts_with("type ")));
    }

    #[tokio::test]
    async fn no_stored_credentials_flags_needs_login_and_types_nothing() {
        let (r, c) = login_rig(Mode::Autonomous).await;
        *r.creds.lock() = None;
        let v = call(&r, &c, "sys_app_login", login_args()).await;
        assert!(is_err(&v));
        assert_eq!(r.runner.status("erp"), (false, true));
        assert!(!r.backend.calls().iter().any(|c| c.starts_with("type ")));
        r.runner.resume("erp");
        assert_eq!(r.runner.status("erp"), (false, false));
    }

    #[tokio::test]
    async fn report_login_needed_sets_the_flag_and_tells_the_ai_to_stop() {
        let r = rig().await;
        let v = call(&r, &conn(Mode::Autonomous), "sys_app_report_login_needed", json!({ "system": "erp", "intent": "讀：要驗證碼", "reason": "要輸入簡訊驗證碼" })).await;
        assert!(!is_err(&v) && out(&v).contains("停下來"));
        assert_eq!(r.runner.status("erp").1, true);
    }

    #[tokio::test]
    async fn input_from_someone_else_pauses_until_resumed() {
        let r = rig().await;
        let c = conn(Mode::Autonomous);
        let a = json!({ "system": "erp", "intent": "讀：捲動" });
        call(&r, &c, "sys_app_scroll", a.clone()).await; // 記下「自己最後一次輸入」
        r.runner.backdate_own_input("erp", 10); // 十秒前
        *r.monitor.idle.lock() = Some(Duration::from_secs(2)); // 但只閒置了 2 秒＝這期間有別人動過
        let calls_before = r.backend.calls().len();
        let v = call(&r, &c, "sys_app_scroll", a.clone()).await;
        assert!(is_err(&v) && out(&v).contains("已暫停"), "{}", out(&v));
        assert_eq!(r.runner.status("erp").0, true);
        assert_eq!(r.backend.calls().len(), calls_before, "暫停時不執行任何動作");
        let again = call(&r, &c, "sys_app_snapshot", a.clone()).await;
        assert!(out(&again).contains("已暫停"), "暫停期間連讀取也擋掉");
        r.runner.resume("erp");
        *r.monitor.idle.lock() = Some(Duration::from_secs(3600));
        assert!(!is_err(&call(&r, &c, "sys_app_scroll", a).await));
        assert!(r.events.lock().iter().any(|e| e.contains("\"manual\":true")), "要發出暫停事件給畫面");
    }

    #[tokio::test]
    async fn an_unknown_idle_time_never_triggers_a_pause() {
        let r = rig().await;
        let c = conn(Mode::Autonomous);
        let a = json!({ "system": "erp", "intent": "讀：捲動" });
        call(&r, &c, "sys_app_scroll", a.clone()).await;
        r.runner.backdate_own_input("erp", 600);
        *r.monitor.idle.lock() = None;
        assert!(!is_err(&call(&r, &c, "sys_app_scroll", a).await));
    }

    #[tokio::test]
    async fn pause_rejects_waiting_approvals() {
        let r = rig().await;
        let c = conn(Mode::ApproveEach);
        call(&r, &c, "sys_app_snapshot", json!({ "system": "erp", "intent": "讀：看" })).await;
        let (runner, c2, pool) = (r.runner.clone(), c.clone(), r.pool.clone());
        let h = tokio::spawn(async move {
            runner.call(&c2, Some(&pool), "sys_app_click", json!({ "system": "erp", "intent": "寫：刪", "element": "0.1" })).await.unwrap()
        });
        while r.approvals.list().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(r.runner.pause("erp"), 1);
        let v = h.await.unwrap();
        assert!(is_err(&v) && out(&v).contains("拒絕"), "{}", out(&v));
        assert!(!r.backend.calls().iter().any(|c| c == "press 0.1"));
    }

    #[tokio::test]
    async fn a_missing_app_is_launched_only_when_the_connection_allows_it() {
        let r = rig().await;
        let mut c = conn(Mode::Autonomous);
        c.app = "Calc".into();
        let a = json!({ "system": "erp", "intent": "讀：看" });
        let v = call(&r, &c, "sys_app_snapshot", a.clone()).await;
        assert!(is_err(&v));
        assert!(r.backend.calls().iter().any(|c| c == "open_app Calc"), "{:?}", r.backend.calls());
        let n = r.backend.calls().iter().filter(|c| c.starts_with("open_app")).count();
        c.launch_app = false;
        call(&r, &c, "sys_app_snapshot", a).await;
        assert_eq!(r.backend.calls().iter().filter(|c| c.starts_with("open_app")).count(), n, "不允許自動啟動就不啟動");
        let _ = UserDecision::Deny;
    }
}
