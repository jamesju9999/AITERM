//! 派出去的 claude 撞到用量上限時：標記「等待額度」，換帳號或等一段時間後
//! 自動送「繼續」。見 docs/superpowers/specs/2026-10-16-agent-quota-resume-design.md。
//!
//! 訊號來自 Claude Code 自己的 HTTP hook（`StopFailure` 帶 `error: "rate_limit"`），
//! 不是比對畫面文字：額度訊息的措辭有十幾種而且隨版本改，hook 的欄位是結構化的。
//! HTTP hook 也不經過 shell，三平台一樣。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use sqlx::SqlitePool;

use crate::pty::PtyManager;
use crate::tasks::store;

/// 沒換帳號時，等多久重試一次。重試的代價只是一次馬上被拒的請求。
pub const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
/// 續跑迴圈多久檢查一次帳號與等待表。
const POLL: Duration = Duration::from_secs(15);
/// 送 Esc 之後等多久再打字：讓額度選單或 Claude 自己的倒數先關掉。
const ESC_SETTLE: Duration = Duration::from_millis(600);
pub const RESUME_TEXT: &str = "繼續";

/// 派工時用 `--settings` 交給 claude 的設定檔內容：三個 HTTP hook 都打到
/// `base`（`http://127.0.0.1:<port>/quota/<secret>/<task_id>`）底下。
/// - `StopFailure` → `/stop-failure`：可能是額度用完。
/// - `PostToolUse`、`Stop` → `/working`：它又在工作了（例如 Claude 自己等到重置後續跑）。
pub fn hook_settings_json(base: &str) -> String {
    let hook = |path: &str| serde_json::json!([{ "hooks": [{ "type": "http", "url": format!("{base}/{path}") }] }]);
    serde_json::json!({
        "hooks": {
            "StopFailure": hook("stop-failure"),
            "PostToolUse": hook("working"),
            "Stop": hook("working"),
        }
    })
    .to_string()
}

/// `StopFailure` 的內容是不是「額度用完」。實測 429 時是 `"error":"rate_limit"`。
pub fn is_quota_failure(body: &serde_json::Value) -> bool {
    body.get("error").and_then(|e| e.as_str()) == Some("rate_limit")
}

/// 從 `.claude.json` 的內容取出目前登入帳號的 id（`oauthAccount.accountUuid`）。
pub fn account_id_from(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.get("oauthAccount")?.get("accountUuid")?.as_str().map(str::to_string)
}

/// Claude Code 記錄登入帳號的檔案：`$CLAUDE_CONFIG_DIR/.claude.json`，沒設就是 `~/.claude.json`。
fn claude_config_file() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join(".claude.json")),
        None => dirs::home_dir().map(|h| h.join(".claude.json")),
    }
}

pub fn current_account() -> Option<String> {
    account_id_from(&std::fs::read_to_string(claude_config_file()?).ok()?)
}

/// 派工指令要接的 `--settings "<path>"`。指令是打進使用者的 shell 的
/// （zsh/bash/fish/pwsh/cmd 都有可能），雙引號在這些 shell 裡對一般路徑都
/// 一樣有效；路徑含 `"`、`$`、`` ` ``、`%` 時各 shell 解讀不同，就不加（退回沒有 hook 的原行為）。
pub fn settings_flag(path: &Path) -> Option<String> {
    let p = path.to_str()?;
    if p.chars().any(|c| matches!(c, '"' | '$' | '`' | '%')) {
        return None;
    }
    Some(format!("--settings \"{p}\""))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nudge {
    AccountChanged,
    Retry,
}

/// 一張等待中的卡片現在該不該續跑。兩邊都讀得到帳號而且不同 → 換帳號了；
/// 否則等滿 `retry_after` 就重試。讀不到帳號（API key 使用者）時只靠重試。
pub fn due(since: Instant, account_at_hit: Option<&str>, now: Instant, current: Option<&str>, retry_after: Duration) -> Option<Nudge> {
    match (account_at_hit, current) {
        (Some(a), Some(c)) if a != c => return Some(Nudge::AccountChanged),
        _ => {}
    }
    (now.saturating_duration_since(since) >= retry_after).then_some(Nudge::Retry)
}

struct Wait {
    since: Instant,
    account: Option<String>,
}

struct Entry {
    tab_id: String,
    pool: SqlitePool,
    wait: Option<Wait>,
}

/// 執行中、由 AITerm 派出去的 claude 卡片，以及其中正在等額度的那些。
#[derive(Default)]
pub struct QuotaState {
    entries: Mutex<HashMap<String, Entry>>,
    server: tokio::sync::OnceCell<Option<(u16, String)>>,
    on_change: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

impl QuotaState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 等待狀態改變時要做的事（正式環境：送 `tasks-updated` 讓畫面重抓）。
    pub fn set_on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        let _ = self.on_change.set(Box::new(f));
    }

    pub fn register(&self, task_id: &str, tab_id: &str, pool: SqlitePool) {
        self.entries.lock().insert(task_id.to_string(), Entry { tab_id: tab_id.to_string(), pool, wait: None });
    }

    pub fn unregister(&self, task_id: &str) {
        self.entries.lock().remove(task_id);
    }

    pub fn is_waiting(&self, task_id: &str) -> bool {
        self.entries.lock().get(task_id).is_some_and(|e| e.wait.is_some())
    }

    /// 標記／解除等待：記憶體、資料表欄位、通知畫面。狀態沒變就什麼都不做
    /// （`PostToolUse` 每個工具呼叫都會來一次）。
    pub async fn set_waiting(&self, task_id: &str, waiting: bool) {
        let pool = {
            let mut entries = self.entries.lock();
            let Some(e) = entries.get_mut(task_id) else { return };
            if e.wait.is_some() == waiting {
                return;
            }
            e.wait = waiting.then(|| Wait { since: Instant::now(), account: current_account() });
            e.pool.clone()
        };
        let at = waiting.then(|| chrono::Utc::now().timestamp());
        if let Err(e) = store::set_quota_wait(&pool, task_id, at).await {
            eprintln!("set_quota_wait {task_id}: {e}");
        }
        if let Some(f) = self.on_change.get() {
            f();
        }
    }

    /// 現在該續跑的卡片：`(task_id, tab_id, 原因)`。
    pub fn due_nudges(&self, now: Instant, current: Option<&str>) -> Vec<(String, String, Nudge)> {
        self.entries
            .lock()
            .iter()
            .filter_map(|(id, e)| {
                let w = e.wait.as_ref()?;
                due(w.since, w.account.as_deref(), now, current, RETRY_AFTER).map(|n| (id.clone(), e.tab_id.clone(), n))
            })
            .collect()
    }

    /// 這張卡片的 hook 網址前綴；接收器第一次需要時才啟動。啟動失敗回 `None`
    /// （派工照常，只是沒有自動續跑）。
    pub async fn hook_base(self: &Arc<Self>, task_id: &str) -> Option<String> {
        let me = self.clone();
        let server = self
            .server
            .get_or_init(|| async move {
                match start_server(me).await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        eprintln!("quota hook server: {e}");
                        None
                    }
                }
            })
            .await;
        let (port, secret) = server.as_ref()?;
        Some(format!("http://127.0.0.1:{port}/quota/{secret}/{task_id}"))
    }

    fn secret(&self) -> Option<&str> {
        self.server.get()?.as_ref().map(|(_, s)| s.as_str())
    }
}

async fn start_server(state: Arc<QuotaState>) -> Result<(u16, String), String> {
    use axum::extract::{Path as AxPath, State};
    use axum::routing::post;

    async fn handle(
        State(state): State<Arc<QuotaState>>,
        AxPath((secret, task_id, kind)): AxPath<(String, String, String)>,
        body: axum::body::Bytes,
    ) -> &'static str {
        // 只接受自己發出去的網址；本機其他程式猜不到密鑰。
        if state.secret() != Some(secret.as_str()) {
            return "{}";
        }
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        match kind.as_str() {
            "stop-failure" if is_quota_failure(&json) => state.set_waiting(&task_id, true).await,
            "working" => state.set_waiting(&task_id, false).await,
            _ => {}
        }
        // Claude Code 會把回應當成 hook 輸出解析；空物件 = 沒有意見。
        "{}"
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let app = axum::Router::new()
        .route("/quota/{secret}/{task_id}/{kind}", post(handle))
        .with_state(state);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("quota hook server stopped: {e}");
        }
    });
    Ok((port, secret))
}

/// 對等額度的分頁送「繼續」：先 Esc 關掉額度選單／取消 Claude 自己的倒數，再打字送出。
pub async fn nudge(pty: &PtyManager, tab_id: &str) -> Result<(), String> {
    pty.write(tab_id, b"\x1b").map_err(|e| e.to_string())?;
    tokio::time::sleep(ESC_SETTLE).await;
    crate::tasks::dispatch::write_then_submit(pty, tab_id, RESUME_TEXT).await
}

/// 常駐的續跑迴圈：換帳號或等滿 `RETRY_AFTER` 的卡片，送「繼續」並解除等待。
/// 又撞上限的話 hook 會再把它標回等待。
pub fn spawn_resumer(state: Arc<QuotaState>, pty: Arc<PtyManager>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            let current = current_account();
            for (task_id, tab_id, reason) in state.due_nudges(Instant::now(), current.as_deref()) {
                eprintln!("quota: resuming {task_id} ({reason:?})");
                if let Err(e) = nudge(&pty, &tab_id).await {
                    eprintln!("quota nudge {task_id}: {e}");
                }
                state.set_waiting(&task_id, false).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[test]
    fn hook_settings_point_every_event_at_the_task_url() {
        let v: serde_json::Value = serde_json::from_str(&hook_settings_json("http://127.0.0.1:9/quota/s/t")).unwrap();
        let url = |ev: &str| v["hooks"][ev][0]["hooks"][0]["url"].as_str().unwrap().to_string();
        assert_eq!(v["hooks"]["StopFailure"][0]["hooks"][0]["type"], "http");
        assert_eq!(url("StopFailure"), "http://127.0.0.1:9/quota/s/t/stop-failure");
        assert_eq!(url("PostToolUse"), "http://127.0.0.1:9/quota/s/t/working");
        assert_eq!(url("Stop"), "http://127.0.0.1:9/quota/s/t/working");
    }

    #[test]
    fn only_rate_limit_counts_as_a_quota_failure() {
        // 實測 StopFailure 的內容（claude 2.1.300 對 429）。
        let hit: serde_json::Value = serde_json::from_str(
            r#"{"hook_event_name":"StopFailure","error":"rate_limit","last_assistant_message":"API Error: Request rejected (429)"}"#,
        )
        .unwrap();
        assert!(is_quota_failure(&hit));
        for other in ["authentication_failed", "overloaded", "server_error"] {
            assert!(!is_quota_failure(&serde_json::json!({ "error": other })), "{other}");
        }
        assert!(!is_quota_failure(&serde_json::json!({})));
    }

    #[test]
    fn account_id_comes_from_oauth_account() {
        assert_eq!(
            account_id_from(r#"{"oauthAccount":{"emailAddress":"a@b","accountUuid":"u-1"},"x":1}"#).as_deref(),
            Some("u-1")
        );
        assert_eq!(account_id_from(r#"{"x":1}"#), None);
        assert_eq!(account_id_from("not json"), None);
    }

    #[test]
    fn settings_flag_quotes_the_path_and_refuses_shell_specials() {
        assert_eq!(
            settings_flag(Path::new("/Users/me/My Project/tasks/t/claude-settings.json")).as_deref(),
            Some(r#"--settings "/Users/me/My Project/tasks/t/claude-settings.json""#)
        );
        for bad in [r#"/a"b"#, "/a$b", "/a`b", r"C:\a%b"] {
            assert_eq!(settings_flag(Path::new(bad)), None, "{bad}");
        }
    }

    #[test]
    fn nudge_on_account_change_or_after_the_retry_interval() {
        let t0 = Instant::now();
        let soon = t0 + Duration::from_secs(30);
        let later = t0 + RETRY_AFTER;
        assert_eq!(due(t0, Some("a"), soon, Some("b"), RETRY_AFTER), Some(Nudge::AccountChanged));
        assert_eq!(due(t0, Some("a"), soon, Some("a"), RETRY_AFTER), None);
        assert_eq!(due(t0, Some("a"), later, Some("a"), RETRY_AFTER), Some(Nudge::Retry));
        // 讀不到帳號：不能當成「換帳號」，只靠重試。
        assert_eq!(due(t0, None, soon, Some("b"), RETRY_AFTER), None);
        assert_eq!(due(t0, Some("a"), soon, None, RETRY_AFTER), None);
    }

    #[tokio::test]
    async fn the_hook_server_marks_and_clears_the_wait() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        let state = QuotaState::new();
        let changes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = changes.clone();
        state.set_on_change(move || {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        state.register(&id, "tab", pool.clone());
        let base = state.hook_base(&id).await.unwrap();
        let client = reqwest::Client::new();
        let post = |path: &str, body: &str| client.post(format!("{base}/{path}")).body(body.to_string()).send();

        // 別種失敗不算。
        post("stop-failure", r#"{"error":"overloaded"}"#).await.unwrap();
        assert!(!state.is_waiting(&id));

        post("stop-failure", r#"{"error":"rate_limit"}"#).await.unwrap();
        assert!(state.is_waiting(&id));
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.quota_wait_at.is_some(), "畫面要看得到");

        // 密鑰不對：不理。
        let wrong = base.replace("/quota/", "/quota/x");
        client.post(format!("{wrong}/working")).body("{}").send().await.unwrap();
        assert!(state.is_waiting(&id));

        post("working", r#"{"hook_event_name":"PostToolUse"}"#).await.unwrap();
        assert!(!state.is_waiting(&id));
        assert_eq!(store::get_task(&pool, &id).await.unwrap().unwrap().quota_wait_at, None);
        assert_eq!(changes.load(std::sync::atomic::Ordering::SeqCst), 2, "只有狀態真的改變才通知");
    }

    #[tokio::test]
    async fn only_waiting_cards_are_due() {
        let pool = mem_pool().await;
        let state = QuotaState::new();
        state.register("a", "tab-a", pool.clone());
        state.register("b", "tab-b", pool.clone());
        state.set_waiting("a", true).await;
        let far = Instant::now() + RETRY_AFTER + Duration::from_secs(1);
        let due = state.due_nudges(far, None);
        assert_eq!(due, vec![("a".to_string(), "tab-a".to_string(), Nudge::Retry)]);
        state.unregister("a");
        assert!(state.due_nudges(far, None).is_empty());
    }
}
