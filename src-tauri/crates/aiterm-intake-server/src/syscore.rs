//! server 上的系統連線執行階段：每張卡、每個系統一個隔離的無頭 Chrome（載入管理員分享的 session）、
//! 寫入閘門、Playwright 代理。卡結束（`McpGuard` 被丟掉）就全部收掉。
//!
//! 這裡只做網頁系統；桌面 App 型連線、配方、定時任務、即時預覽與接手不在 server 版（見 M3／M4 spec）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aiterm_intake_core::board_store::{self as bs, BoardSystem};
use aiterm_intake_core::host::Notifier;
use aiterm_systems::approvals::{Approvals, PendingView};
use aiterm_systems::browser::{self, RunningBrowser};
use aiterm_systems::cdp::{run_gate, Cdp, GateCtx, GateEvent};
use aiterm_systems::gate::looks_logged_out;
use aiterm_systems::proxy::{fetch_catalog, Proxy, Upstream};
use aiterm_systems::types::{Mode, SystemConnection};
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::state::AppState;

/// 這張卡、這個系統的一次執行。
pub struct SysRun {
    browser: tokio::sync::Mutex<RunningBrowser>,
    gate: tokio::task::JoinHandle<()>,
    proxy: Arc<Proxy>,
    ctx: GateCtx,
    needs_login: AtomicBool,
    dir: PathBuf,
    system_id: String,
    system_name: String,
    owner: Owner,
    /// 即時預覽與接手的來源（第一位觀看者出現才真的開始串流）。
    pub source: Arc<crate::syslive::LiveSource>,
}

/// 一筆待核准請求屬於哪張卡（核准清單依專案過濾、卡結束時把它的拒絕掉）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub project_id: String,
    pub card_id: String,
    pub agent_id: Option<String>,
}

#[derive(Default)]
pub struct SysState {
    runs: tokio::sync::Mutex<HashMap<(String, String), Arc<SysRun>>>,
    pub approvals: Arc<Approvals>,
    owners: Mutex<HashMap<String, Owner>>,
    catalog: tokio::sync::OnceCell<Vec<Value>>,
    /// 遠端登入中的瀏覽器（每個系統一個）。
    logins: tokio::sync::Mutex<HashMap<String, Arc<LoginRun>>>,
}

/// 一個遠端登入：server 上的隔離瀏覽器，管理員透過串流在上面登入。
pub struct LoginRun {
    browser: tokio::sync::Mutex<RunningBrowser>,
    dir: PathBuf,
    pub source: Arc<crate::syslive::LiveSource>,
}

/// 待核准清單的一筆：核准內容加上是哪個專案、哪張卡、哪位 Agent。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingSys {
    #[serde(flatten)]
    pub view: PendingView,
    pub system_name: String,
    pub card_id: String,
    pub agent_id: Option<String>,
}

impl SysState {
    /// 某專案等核准的系統請求。
    pub fn pending_for(&self, project_id: &str, names: &HashMap<String, String>) -> Vec<PendingSys> {
        let owners = self.owners.lock();
        self.approvals
            .list()
            .into_iter()
            .filter_map(|v| {
                let o = owners.get(&v.id)?;
                (o.project_id == project_id).then(|| PendingSys { system_name: names.get(&v.pending.system_id).cloned().unwrap_or_else(|| v.pending.system_id.clone()), card_id: o.card_id.clone(), agent_id: o.agent_id.clone(), view: v })
            })
            .collect()
    }

    pub fn owner_of(&self, approval_id: &str) -> Option<Owner> {
        self.owners.lock().get(approval_id).cloned()
    }
}

fn connection_of(s: &BoardSystem) -> SystemConnection {
    let mode = match s.mode.as_str() {
        "approve_each" => Mode::ApproveEach,
        "autonomous" => Mode::Autonomous,
        _ => Mode::ReadOnly,
    };
    SystemConnection { id: s.id.clone(), name: s.name.clone(), start_url: s.start_url.clone(), allowed_hosts: s.hosts(), mode, enabled: s.enabled, ..Default::default() }
}

fn run_dir(st: &AppState, card_id: &str, system_id: &str) -> PathBuf {
    st.data_dir.join("run").join(card_id).join("sys").join(system_id)
}

/// 把管理員分享的 session 載進這個隔離的瀏覽器（瀏覽器層級的 `Storage.setCookies`）。
async fn inject_cookies(ws_url: &str, cookies: &[Value]) -> Result<(), String> {
    let (cdp, _events) = Cdp::connect(ws_url).await?;
    cdp.call("Storage.setCookies", json!({ "cookies": cookies }), None).await.map(|_| ()).map_err(|e| format!("載入 session 失敗：{e}"))
}

/// 這張卡的這個系統已經在跑就直接用，否則啟動：隔離的無頭 Chrome＋載入 session＋閘門＋Playwright 代理。
pub async fn ensure_running(st: &Arc<AppState>, ctx: &crate::dbmcp::McpCtx, system_id: &str) -> Result<Arc<SysRun>, String> {
    if !ctx.systems.iter().any(|s| s == system_id) {
        return Err(format!("這個工作沒有被授權使用系統「{system_id}」（先用 sys_list 看可用的系統）"));
    }
    let key = (ctx.card_id.clone(), system_id.to_string());
    let mut runs = st.sys.runs.lock().await;
    if let Some(r) = runs.get(&key) {
        if r.browser.lock().await.is_alive() && !r.gate.is_finished() {
            return Ok(r.clone());
        }
        r.proxy.close().await;
        r.gate.abort();
        runs.remove(&key);
    }
    let system = bs::get_system(&st.pools.intake, system_id).await.map_err(|e| e.to_string())?.ok_or("這個系統連線已經不存在")?;
    if !system.enabled {
        return Err("這個系統連線已停用".into());
    }
    let cookies = crate::sysconf::load_session(st, system_id).ok_or("這個系統還沒有分享登入 session。請管理員先在桌面版登入這個系統，再按「分享 session」。")?;
    let dir = run_dir(st, &ctx.card_id, system_id);
    let browser = browser::launch_with(&dir, "profile", "about:blank", true).await?;
    inject_cookies(&browser.ws_url, &cookies).await?;

    let last_denied: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let owner = Owner { project_id: ctx.project_id.clone(), card_id: ctx.card_id.clone(), agent_id: ctx.agent_id.clone() };
    let (me, ld) = (st.clone(), last_denied.clone());
    let emit: Arc<dyn Fn(GateEvent) + Send + Sync> = Arc::new(move |e| match e {
        GateEvent::NeedApproval { id, .. } => {
            me.sys.owners.lock().insert(id, owner.clone());
            me.events.changed();
        }
        GateEvent::Resolved { id } => {
            me.sys.owners.lock().remove(&id);
            me.events.changed();
        }
        GateEvent::Denied { method, url, .. } => {
            *ld.lock() = Some(format!("{method} {url}"));
            me.events.changed();
        }
    });
    let mut gctx = GateCtx::new(connection_of(&system), st.sys.approvals.clone(), emit);
    gctx.audit = Some(st.pools.intake.clone());
    let ws = browser.ws_url.clone();
    let gate_ctx = gctx.clone();
    let gate = tokio::spawn(async move {
        if let Err(e) = run_gate(&ws, gate_ctx).await {
            log::warn!("系統連線的閘門結束：{e}");
        }
    });
    tokio::time::sleep(Duration::from_millis(600)).await;
    // 先整理好分頁（關掉空白頁、開起始頁）再讓 Playwright 連上：它連上之後分頁就不能再被關掉，不然第一次導覽會遇到「瀏覽器已關閉」。
    browser::show_start_page(&browser.ws_url, &system.start_url).await;

    let out = dir.join("output");
    std::fs::create_dir_all(&out).ok();
    let upstream = Upstream::spawn_with(Some(&browser.cdp_http()), &out, false).await?;
    let proxy = Proxy::start(system_id, upstream, st.sys.approvals.clone(), gctx.intent.clone(), gctx.manual.clone(), last_denied).await?;
    proxy.warm_up().await;
    let source = crate::syslive::LiveSource::new(browser.ws_url.clone(), None, "agent", Some(gctx.manual.clone()));
    let owner = Owner { project_id: ctx.project_id.clone(), card_id: ctx.card_id.clone(), agent_id: ctx.agent_id.clone() };
    let run = Arc::new(SysRun { browser: tokio::sync::Mutex::new(browser), gate, proxy: Arc::new(proxy), ctx: gctx, needs_login: AtomicBool::new(false), dir, system_id: system_id.to_string(), system_name: system.name.clone(), owner, source });
    runs.insert(key, run.clone());
    Ok(run)
}

impl SysState {
    /// 測試用：直接指定工具目錄，不必真的啟動 Playwright MCP 去讀。
    pub fn set_catalog(&self, tools: Vec<Value>) {
        let _ = self.catalog.set(tools);
    }
}

/// 取得 `sys_*` 工具目錄（要啟動一次 Playwright MCP 去讀；快取起來）。
pub async fn catalog(st: &Arc<AppState>) -> Result<Vec<Value>, String> {
    st.sys
        .catalog
        .get_or_try_init(|| async {
            let dir = st.data_dir.join("run").join("_catalog");
            std::fs::create_dir_all(&dir).ok();
            let r = fetch_catalog(&dir).await;
            let _ = std::fs::remove_dir_all(&dir);
            r
        })
        .await
        .cloned()
}

/// `sys_list`：這張卡能用的系統。
pub async fn list(st: &AppState, ctx: &crate::dbmcp::McpCtx) -> Result<String, String> {
    let mut out = Vec::new();
    for id in &ctx.systems {
        if let Some(s) = bs::get_system(&st.pools.intake, id).await.map_err(|e| e.to_string())? {
            let has_session = bs::get_system_session(&st.pools.intake, id).await.map_err(|e| e.to_string())?.is_some();
            out.push(json!({"id": s.id, "name": s.name, "start_url": s.start_url, "mode": s.mode, "enabled": s.enabled, "has_session": has_session}));
        }
    }
    if out.is_empty() {
        return Ok("沒有可用的系統連線。".into());
    }
    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
}

/// 一次 `sys_*` 工具呼叫。
pub async fn call(st: &Arc<AppState>, ctx: &crate::dbmcp::McpCtx, tool: &str, args: Value) -> Result<Value, String> {
    let system_id = args["system"].as_str().unwrap_or("").to_string();
    if system_id.is_empty() {
        return Err("缺少 system 參數（先用 sys_list 查看可用的系統 id）".into());
    }
    let run = ensure_running(st, ctx, &system_id).await?;
    let system = bs::get_system(&st.pools.intake, &system_id).await.map_err(|e| e.to_string())?.ok_or("這個系統連線已經不存在")?;
    if !system.enabled {
        close_system(st, &system_id).await;
        return Err("這個系統連線已停用".into());
    }
    *run.ctx.conn.lock() = connection_of(&system); // 模式改了立刻生效
    let mut res = run.proxy.call(tool, args).await?;
    let text = res["content"].as_array().map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
    let conn = connection_of(&system);
    let logged_out = looks_logged_out(&conn, &text);
    if run.needs_login.swap(logged_out, Ordering::SeqCst) != logged_out {
        st.events.changed();
    }
    if logged_out {
        let note = "注意：頁面看起來是登入頁，這個系統的登入 session 可能已失效。請停下來回報「需要重新分享 session」；不要嘗試自己填入帳號密碼。";
        if let Some(a) = res["content"].as_array_mut() {
            a.push(json!({ "type": "text", "text": note }));
        }
    }
    Ok(res)
}

/// 哪些系統目前需要重新分享 session（執行中偵測到被導回登入頁）。
pub async fn needs_login(st: &AppState, project_id: &str) -> Vec<String> {
    let _ = project_id;
    st.sys.runs.lock().await.values().filter(|r| r.needs_login.load(Ordering::SeqCst)).map(|r| r.system_id.clone()).collect()
}

async fn shut(r: &Arc<SysRun>) {
    r.source.shutdown("這張卡已經結束").await;
    r.proxy.close().await;
    r.gate.abort();
    r.browser.lock().await.close().await;
    let _ = std::fs::remove_dir_all(&r.dir);
}

/// 這張卡結束：它所有的瀏覽器、代理、等待中的核准一併收掉，暫存資料夾刪除。
pub async fn close_card(st: &Arc<AppState>, card_id: &str) {
    // 先把這張卡等核准的請求全部拒絕（沒有人在等了）。
    let mine: Vec<String> = st.sys.owners.lock().iter().filter(|(_, o)| o.card_id == card_id).map(|(id, _)| id.clone()).collect();
    for id in mine {
        st.sys.approvals.decide(&id, false);
        st.sys.owners.lock().remove(&id);
    }
    let taken: Vec<Arc<SysRun>> = {
        let mut runs = st.sys.runs.lock().await;
        let keys: Vec<(String, String)> = runs.keys().filter(|(c, _)| c == card_id).cloned().collect();
        keys.into_iter().filter_map(|k| runs.remove(&k)).collect()
    };
    for r in &taken {
        shut(r).await;
    }
    let _ = std::fs::remove_dir_all(st.data_dir.join("run").join(card_id).join("sys"));
    if !taken.is_empty() {
        st.events.changed();
    }
}

/// 系統被刪除或停用：所有卡裡這個系統的瀏覽器一併關掉。
pub async fn close_system(st: &Arc<AppState>, system_id: &str) {
    let taken: Vec<Arc<SysRun>> = {
        let mut runs = st.sys.runs.lock().await;
        let keys: Vec<(String, String)> = runs.keys().filter(|(_, s)| s == system_id).cloned().collect();
        keys.into_iter().filter_map(|k| runs.remove(&k)).collect()
    };
    for r in &taken {
        st.sys.approvals.reject_all(system_id);
        shut(r).await;
    }
    cancel_login(st, system_id).await;
}

// ---- 即時預覽（Agent 執行中） ----

/// 執行中、可以觀看的一個（卡片, 系統）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveRunInfo {
    pub card_id: String,
    pub system_id: String,
    pub system_name: String,
    pub agent_id: Option<String>,
    /// 是否被接手中。
    pub manual: bool,
    pub viewers: usize,
}

/// 某專案目前有瀏覽器在跑的系統。
pub async fn live_runs(st: &AppState, project_id: &str) -> Vec<LiveRunInfo> {
    st.sys
        .runs
        .lock()
        .await
        .iter()
        .filter(|(_, r)| r.owner.project_id == project_id)
        .map(|((card, _), r)| LiveRunInfo { card_id: card.clone(), system_id: r.system_id.clone(), system_name: r.system_name.clone(), agent_id: r.owner.agent_id.clone(), manual: r.source.is_manual(), viewers: r.source.viewers() })
        .collect()
}

/// 某張卡的某個系統的觀看來源。
pub async fn live_source(st: &AppState, card_id: &str, system_id: &str) -> Option<(Arc<crate::syslive::LiveSource>, String)> {
    let runs = st.sys.runs.lock().await;
    runs.get(&(card_id.to_string(), system_id.to_string())).map(|r| (r.source.clone(), r.system_name.clone()))
}

// ---- 遠端登入 ----

/// 閒置多久自動關閉遠端登入。
pub const LOGIN_IDLE: Duration = Duration::from_secs(15 * 60);

/// 開始遠端登入：在 server 開一個全新的隔離瀏覽器（固定 1280×800）到起始網址。同一個系統同時只能有一個。
pub async fn start_login(st: &Arc<AppState>, system_id: &str) -> Result<(), String> {
    let system = bs::get_system(&st.pools.intake, system_id).await.map_err(|e| e.to_string())?.ok_or("找不到這個系統連線")?;
    if !system.enabled {
        return Err("這個系統連線已停用".into());
    }
    let mut logins = st.sys.logins.lock().await;
    if logins.contains_key(system_id) {
        return Err("這個系統已經有人在遠端登入了".into());
    }
    let dir = st.data_dir.join("run").join(format!("login-{system_id}"));
    let _ = std::fs::remove_dir_all(&dir);
    let browser = browser::launch_with(&dir, "profile", &system.start_url, true).await?;
    let source = crate::syslive::LiveSource::new(browser.ws_url.clone(), Some((1280, 800)), "login", None);
    let run = Arc::new(LoginRun { browser: tokio::sync::Mutex::new(browser), dir, source });
    logins.insert(system_id.to_string(), run.clone());
    // 閒置逾時：沒人完成也沒人取消就自動收掉（避免留著一個登入著的瀏覽器）。
    let (st2, id) = (st.clone(), system_id.to_string());
    tokio::spawn(async move {
        tokio::time::sleep(LOGIN_IDLE).await;
        let same = st2.sys.logins.lock().await.get(&id).is_some_and(|r| Arc::ptr_eq(r, &run));
        if same {
            cancel_login(&st2, &id).await;
        }
    });
    Ok(())
}

/// 遠端登入的觀看來源。
pub async fn login_source(st: &AppState, system_id: &str) -> Option<Arc<crate::syslive::LiveSource>> {
    st.sys.logins.lock().await.get(system_id).map(|r| r.source.clone())
}

async fn close_login_run(run: &Arc<LoginRun>, why: &str) {
    run.source.shutdown(why).await;
    run.browser.lock().await.close().await;
    let _ = std::fs::remove_dir_all(&run.dir);
}

/// 放棄遠端登入（或閒置逾時、系統被刪）。
pub async fn cancel_login(st: &Arc<AppState>, system_id: &str) {
    let run = st.sys.logins.lock().await.remove(system_id);
    if let Some(r) = run {
        close_login_run(&r, "遠端登入已結束").await;
        st.events.changed();
    }
}

/// 完成遠端登入：取出瀏覽器裡的 cookie、篩選白名單網域、加密保存成這個系統的 session，然後關掉瀏覽器。
/// 沒有任何白名單網域的 cookie 時回錯誤並**保持登入開著**，讓管理員繼續登入。
pub async fn finish_login(st: &Arc<AppState>, system_id: &str, by: &str) -> Result<(), String> {
    let run = st.sys.logins.lock().await.get(system_id).cloned().ok_or("這個系統沒有進行中的遠端登入")?;
    let live = run.source.live().await.ok_or("串流還沒就緒")?;
    let cookies = live.cookies().await?;
    let system = bs::get_system(&st.pools.intake, system_id).await.map_err(|e| e.to_string())?.ok_or("系統連線已經不存在")?;
    crate::sysconf::store_session(st, &system, &cookies, by).await?;
    st.sys.logins.lock().await.remove(system_id);
    close_login_run(&run, "登入完成，session 已保存").await;
    crate::audit::record(&st.pools.server, by, "system.login.finish", &system.name).await;
    st.events.changed();
    Ok(())
}
