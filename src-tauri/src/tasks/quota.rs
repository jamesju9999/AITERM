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

/// `Stop` 之後多久沒動靜就算「在等人回覆」。完成標記那一輪結束後卡片會在這段時間內
/// 結案（解除登記），所以不會被誤判。
pub const REPLY_GRACE: Duration = Duration::from_secs(8);

/// 派工時用 `--settings` 交給 claude 的設定檔內容：HTTP hook 都打到
/// `base`（`http://127.0.0.1:<port>/quota/<secret>/<task_id>`）底下。
/// - `StopFailure` → `/stop-failure`：可能是額度用完。
/// - `Stop` → `/turn-end`：結束一輪回覆；之後沒動靜就是在等人回覆。
/// - `PostToolUse`、`UserPromptSubmit` → `/working`：又在工作了（Claude 自己等到額度重置後續跑、或使用者回覆了）。
pub fn hook_settings_json(base: &str) -> String {
    hook_settings_json_with(base, &[])
}

/// 同 `hook_settings_json`，另外把 `plugins_off` 這些外掛關掉（精簡啟動）。
pub fn hook_settings_json_with(base: &str, plugins_off: &[String]) -> String {
    let hook = |path: &str| serde_json::json!([{ "hooks": [{ "type": "http", "url": format!("{base}/{path}") }] }]);
    let mut v = serde_json::json!({
        "hooks": {
            "StopFailure": hook("stop-failure"),
            "PostToolUse": hook("working"),
            "UserPromptSubmit": hook("working"),
            "Stop": hook("turn-end"),
        }
    });
    if !plugins_off.is_empty() {
        let off: serde_json::Map<String, serde_json::Value> = plugins_off.iter().map(|p| (p.clone(), false.into())).collect();
        v["enabledPlugins"] = off.into();
    }
    v.to_string()
}

/// 在派工 settings 的 `env` 加一個環境變數（例如等核准時把 MCP 工具逾時拉長）。
pub fn settings_with_env(settings_json: &str, key: &str, value: &str) -> String {
    let mut v: serde_json::Value = serde_json::from_str(settings_json).unwrap_or_else(|_| serde_json::json!({}));
    v["env"][key] = value.into();
    v.to_string()
}

/// 寫入要核准時，db_query 會掛著等人按核准：Claude 的 MCP 工具逾時拉到 24 小時。
pub const APPROVAL_TOOL_TIMEOUT_MS: &str = "86400000";

/// 精簡啟動加在指令後面的旗標：只用 `--mcp-config` 給的 MCP 伺服器、不載入技能。
/// 實測（claude 2.1.300，`-p` 模式第一次呼叫）：預設約 25K token，加上這兩個再關掉
/// 外掛約 11.7K。`--bare` 不能用：它連 `--settings` 裡的 hook 都略過、也不讀 OAuth。
pub const LEAN_FLAGS: &str = "--strict-mcp-config --disable-slash-commands";

/// 使用者在 `~/.claude/settings.json` 啟用的外掛（精簡啟動時逐一關掉）。
pub fn user_enabled_plugins(settings_json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(settings_json)
        .ok()
        .and_then(|v| v.get("enabledPlugins").and_then(|p| p.as_object()).cloned())
        .map(|m| m.into_iter().filter(|(_, on)| on.as_bool() == Some(true)).map(|(k, _)| k).collect())
        .unwrap_or_default()
}

/// 通知「在等你回覆」「有問題要你決定」時用的卡片資訊。
#[derive(Debug, Clone, Default)]
pub struct ReplyLabel {
    pub title: String,
    pub project_name: String,
    pub project_id: String,
    pub agent_id: Option<String>,
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
    path_flag("--settings", path)
}

/// `<flag> "<path>"`，規則同 `settings_flag`（`--mcp-config` 也用它）。
pub fn path_flag(flag: &str, path: &Path) -> Option<String> {
    let p = path.to_str()?;
    if p.chars().any(|c| matches!(c, '"' | '$' | '`' | '%')) {
        return None;
    }
    Some(format!("{flag} \"{p}\""))
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
    label: ReplyLabel,
    /// 每次 `turn-end` 加一；寬限期滿時還是同一輪、而且沒被 `working` 清掉才算在等。
    turn: u64,
    turn_pending: bool,
    reply_wait: bool,
    /// 有還沒回答的 `ask_human` 問題。
    asking: bool,
    /// Codex 卡片：ask_human 掛著等答案（沒有對話可以把答案打進去）。
    blocking_ask: bool,
    /// 問題 id → 等答案的提問請求。解除登記時一起丟掉，等待端就會收到「已取消」。
    waiters: HashMap<i64, tokio::sync::oneshot::Sender<String>>,
    /// 這張卡的 Agent 的資料庫技能（`None`＝沒裝，db 工具不出現）。
    db: Option<crate::tasks::db_skill::DbSkill>,
    /// 寫入核准的問題 id → 等原始答案的 db_query 呼叫。
    approvals: HashMap<i64, tokio::sync::oneshot::Sender<String>>,
    /// 這張卡的自訂技能（名稱、資料夾）：讀到它的 SKILL.md 就記一筆使用。
    skills: Vec<(String, std::path::PathBuf)>,
}

/// 寫入核准問題的兩個選項。
pub const APPROVE: &str = "核准執行";
pub const REJECT: &str = "拒絕";

/// AITerm 已設定的資料庫連線（不含帳密）。
#[derive(Debug, Clone)]
pub struct DbConnInfo {
    pub id: String,
    pub name: String,
    pub db_type: String,
    pub database: String,
}

/// db 工具的執行結果；`rows` 是 db_query 的列數。
pub struct DbOutput {
    pub text: String,
    pub rows: Option<i64>,
}

/// 實際查資料庫的地方（正式環境接 `mcp_server::db_ops`；測試換成假的）。
pub struct DbBackend {
    pub connections: Box<dyn Fn() -> Vec<DbConnInfo> + Send + Sync>,
    pub run: Box<dyn Fn(crate::tasks::ask::DbCall) -> futures_util::future::BoxFuture<'static, Result<DbOutput, String>> + Send + Sync>,
}

/// 執行中、由 AITerm 派出去的 claude 卡片，以及其中正在等額度或等人回覆的那些。
pub struct QuotaState {
    entries: Mutex<HashMap<String, Entry>>,
    server: tokio::sync::OnceCell<Option<(u16, String)>>,
    on_change: OnceLock<Box<dyn Fn() + Send + Sync>>,
    on_reply_wait: OnceLock<Box<dyn Fn(&ReplyLabel) + Send + Sync>>,
    on_question: OnceLock<Box<dyn Fn(&ReplyLabel, &crate::tasks::questions::QuestionRow) + Send + Sync>>,
    db_backend: OnceLock<DbBackend>,
    reply_grace: Duration,
}

impl QuotaState {
    pub fn new() -> Arc<Self> {
        Self::with_reply_grace(REPLY_GRACE)
    }

    pub fn with_reply_grace(reply_grace: Duration) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::default(),
            server: tokio::sync::OnceCell::new(),
            on_change: OnceLock::new(),
            on_reply_wait: OnceLock::new(),
            on_question: OnceLock::new(),
            db_backend: OnceLock::new(),
            reply_grace,
        })
    }

    /// 有新問題時要做的事（正式環境：送 `agent-question`、發到 Telegram）。
    pub fn set_on_question(
        &self,
        f: impl Fn(&ReplyLabel, &crate::tasks::questions::QuestionRow) + Send + Sync + 'static,
    ) {
        let _ = self.on_question.set(Box::new(f));
    }

    /// 這張卡的 `ask_human` MCP 端點網址；接收器第一次需要時才啟動。
    pub async fn mcp_url(self: &Arc<Self>, task_id: &str) -> Option<String> {
        let base = self.hook_base(task_id).await?;
        Some(base.replacen("/quota/", "/mcp/", 1))
    }

    /// 等決策：有沒有還沒回答的問題。變了就通知畫面。
    pub fn set_asking(&self, task_id: &str, asking: bool) {
        let changed = {
            let mut entries = self.entries.lock();
            match entries.get_mut(task_id) {
                Some(e) if e.asking != asking => {
                    e.asking = asking;
                    true
                }
                _ => false,
            }
        };
        if changed {
            if let Some(f) = self.on_change.get() {
                f();
            }
        }
    }

    /// 等決策時，完成標記先不算結案（它還沒拿到答案）。
    pub fn is_holding_marker(&self, task_id: &str) -> bool {
        self.entries.lock().get(task_id).is_some_and(|e| e.asking)
    }

    /// `ask_human`：建立問題、進入等決策、通知。回傳要回給 Claude 的文字與是否是錯誤。
    async fn ask(&self, task_id: &str, q: &crate::tasks::questions::NewQuestion) -> (String, bool) {
        let Some((pool, label)) = self.entries.lock().get(task_id).map(|e| (e.pool.clone(), e.label.clone())) else {
            return ("這張卡已經結束，不能再提問。".into(), true);
        };
        // 先進入等決策：問題一建好就可能被看見，那時已經不能判卡住。
        self.set_asking(task_id, true);
        match crate::tasks::questions::create(&pool, task_id, label.agent_id.as_deref(), q).await {
            Ok(row) => {
                // 先登記等待者再通知：通知之後馬上就可能有人回答。
                let waiter = {
                    let mut entries = self.entries.lock();
                    entries.get_mut(task_id).filter(|e| e.blocking_ask).map(|e| {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        e.waiters.insert(row.id, tx);
                        rx
                    })
                };
                if let Some(f) = self.on_question.get() {
                    f(&label, &row);
                }
                match waiter {
                    Some(rx) => match rx.await {
                        Ok(answer) => (answer, false),
                        Err(_) => ("問題已取消（卡片已結束），不要再等這個答案。".into(), true),
                    },
                    None => (crate::tasks::questions::ask_reply(&row), false),
                }
            }
            Err(e) => {
                if !crate::tasks::questions::has_open(&pool, task_id).await.unwrap_or(false) {
                    self.set_asking(task_id, false);
                }
                (format!("提問失敗：{e}"), true)
            }
        }
    }

    /// 開始等人回覆時要做的事（正式環境：送 `agent-reply-wait` 讓前端發通知）。
    pub fn set_on_reply_wait(&self, f: impl Fn(&ReplyLabel) + Send + Sync + 'static) {
        let _ = self.on_reply_wait.set(Box::new(f));
    }

    /// 等待狀態改變時要做的事（正式環境：送 `tasks-updated` 讓畫面重抓）。
    pub fn set_on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        let _ = self.on_change.set(Box::new(f));
    }

    pub fn register(&self, task_id: &str, tab_id: &str, pool: SqlitePool, label: ReplyLabel) {
        self.entries.lock().insert(
            task_id.to_string(),
            Entry {
                tab_id: tab_id.to_string(),
                pool,
                wait: None,
                label,
                turn: 0,
                turn_pending: false,
                reply_wait: false,
                asking: false,
                blocking_ask: false,
                waiters: HashMap::new(),
                db: None,
                approvals: HashMap::new(),
                skills: Vec::new(),
            },
        );
    }

    /// 資料庫實際執行的地方。
    pub fn set_db_backend(&self, backend: DbBackend) {
        let _ = self.db_backend.set(backend);
    }

    /// 這張卡的 Agent 的資料庫技能。在 `register` 之後呼叫。
    pub fn set_db_skill(&self, task_id: &str, skill: Option<crate::tasks::db_skill::DbSkill>) {
        if let Some(e) = self.entries.lock().get_mut(task_id) {
            e.db = skill;
        }
    }

    /// 這張卡的自訂技能。在 `register` 之後呼叫。
    pub fn set_custom_skills(&self, task_id: &str, skills: Vec<(String, std::path::PathBuf)>) {
        if let Some(e) = self.entries.lock().get_mut(task_id) {
            e.skills = skills;
        }
    }

    /// PostToolUse：Read 了某個技能的 SKILL.md → 記一筆技能使用（畫面上技能列會亮）。
    async fn tool_used(&self, task_id: &str, json: &serde_json::Value) {
        if json.get("tool_name").and_then(|v| v.as_str()) != Some("Read") {
            return;
        }
        let Some(file) = json.pointer("/tool_input/file_path").and_then(|v| v.as_str()) else { return };
        let hit = self.entries.lock().get(task_id).and_then(|e| {
            crate::tasks::custom_skills::skill_of_read(&e.skills, file).map(|skill| (e.pool.clone(), e.label.agent_id.clone(), skill))
        });
        if let Some((pool, agent_id, skill)) = hit {
            if let Err(e) = crate::tasks::custom_skills::record_use(&pool, task_id, agent_id.as_deref(), &skill).await {
                eprintln!("record skill use {task_id}: {e}");
            }
            self.changed();
        }
    }

    fn has_db(&self, task_id: &str) -> bool {
        self.entries.lock().get(task_id).is_some_and(|e| e.db.is_some())
    }

    /// 這個問題是不是有 db_query 在等核准。
    pub fn has_approval(&self, task_id: &str, question_id: i64) -> bool {
        self.entries.lock().get(task_id).is_some_and(|e| e.approvals.contains_key(&question_id))
    }

    /// 把使用者的原始答案交給等核准的 db_query。
    pub fn give_approval(&self, task_id: &str, question_id: i64, answer: &str) -> bool {
        let tx = self.entries.lock().get_mut(task_id).and_then(|e| e.approvals.remove(&question_id));
        tx.is_some_and(|tx| tx.send(answer.to_string()).is_ok())
    }

    fn changed(&self) {
        if let Some(f) = self.on_change.get() {
            f();
        }
    }

    /// 資料庫技能的工具呼叫：檢查權限（需要時等使用者核准）、執行、記進 `db_queries`。
    /// 回傳要給 Agent 的文字與是否是錯誤。
    pub(crate) async fn db(&self, task_id: &str, call: crate::tasks::ask::DbCall) -> (String, bool) {
        use crate::tasks::ask::DbCall;
        use crate::tasks::db_skill::{self as skill, Decision, NewQuery};
        let Some((pool, label, Some(db))) = self.entries.lock().get(task_id).map(|e| (e.pool.clone(), e.label.clone(), e.db.clone())) else {
            return ("這張卡沒有資料庫技能。".into(), true);
        };
        let Some(backend) = self.db_backend.get() else {
            return ("AITerm 的資料庫功能還沒準備好。".into(), true);
        };
        let conns = (backend.connections)();
        let mode_label = match db.mode {
            skill::DbMode::Read => "唯讀",
            skill::DbMode::Approve => "寫入需核准",
            skill::DbMode::Write => "可寫入",
        };
        let (connection_id, sql) = match &call {
            DbCall::ListConnections => {
                let list: Vec<serde_json::Value> = conns
                    .iter()
                    .filter(|c| db.connections.contains(&c.id))
                    .map(|c| serde_json::json!({ "id": c.id, "name": c.name, "db_type": c.db_type, "database": c.database, "permission": mode_label }))
                    .collect();
                return (serde_json::to_string_pretty(&list).unwrap_or_default(), false);
            }
            DbCall::ListSchemas { connection_id } | DbCall::ListTables { connection_id, .. } | DbCall::TableSchema { connection_id, .. } => {
                (connection_id.clone(), None)
            }
            DbCall::Query { connection_id, sql } => (connection_id.clone(), Some(sql.clone())),
        };
        let name = conns.iter().find(|c| c.id == connection_id).map(|c| c.name.clone()).unwrap_or_else(|| connection_id.clone());
        let Some(sql) = sql else {
            // 結構查詢不改資料，只檢查連線有沒有開放。
            if !db.connections.contains(&connection_id) {
                return ("這個連線沒有開放給你，先用 db_list_connections 看可以用哪些。".into(), true);
            }
            return match (backend.run)(call).await {
                Ok(out) => (out.text, false),
                Err(e) => (e, true),
            };
        };
        let q = NewQuery {
            task_id,
            agent_id: label.agent_id.as_deref(),
            connection_id: &connection_id,
            connection_name: &name,
            sql: &sql,
            decision: "allowed",
        };
        let id = match skill::authorize(&db, &connection_id, &sql) {
            Decision::Allow => skill::start_query(&pool, &q).await.ok(),
            Decision::Deny(reason) => {
                let _ = skill::record_denied(&pool, &q, &reason).await;
                self.changed();
                return (reason, true);
            }
            Decision::NeedApproval => {
                // 等核准的這段也記下來（pending），畫面才看得出 Agent 卡在等你。核准後同一筆接著執行。
                let id = skill::start_query(&pool, &NewQuery { decision: "pending", ..q }).await.ok();
                self.changed();
                match self.wait_for_approval(task_id, &pool, &label, &name, &sql).await {
                    Ok(()) => {
                        if let Some(id) = id {
                            let _ = skill::set_decision(&pool, id, "approved").await;
                        }
                        id
                    }
                    Err(reason) => {
                        if let Some(id) = id {
                            let _ = skill::set_decision(&pool, id, "denied").await;
                            let _ = skill::finish_query(&pool, id, None, 0, Some(&reason)).await;
                        }
                        self.changed();
                        return (reason, true);
                    }
                }
            }
        };
        self.changed();
        let started = Instant::now();
        let result = (backend.run)(call).await;
        let ms = started.elapsed().as_millis() as i64;
        if let Some(id) = id {
            let (rows, err) = match &result {
                Ok(out) => (out.rows, None),
                Err(e) => (None, Some(e.as_str())),
            };
            let _ = skill::finish_query(&pool, id, rows, ms, err).await;
        }
        self.changed();
        match result {
            Ok(out) => (out.text, false),
            Err(e) => (e, true),
        }
    }

    /// 建一個核准問題（收件匣＋Telegram）並等答案。核准回 Ok，其他回拒絕原因。
    async fn wait_for_approval(
        &self,
        task_id: &str,
        pool: &SqlitePool,
        label: &ReplyLabel,
        conn_name: &str,
        sql: &str,
    ) -> Result<(), String> {
        let q = crate::tasks::questions::NewQuestion {
            question: format!("要允許在資料庫「{conn_name}」執行這句寫入嗎？"),
            options: vec![APPROVE.into(), REJECT.into()],
            recommended: None,
            context: Some(sql.to_string()),
        };
        // 先進入等決策：問題一建好就可能被看見，那時已經不能判卡住。
        self.set_asking(task_id, true);
        let row = match crate::tasks::questions::create(pool, task_id, label.agent_id.as_deref(), &q).await {
            Ok(row) => row,
            Err(e) => {
                if !crate::tasks::questions::has_open(pool, task_id).await.unwrap_or(false) {
                    self.set_asking(task_id, false);
                }
                return Err(format!("建立核准問題失敗：{e}"));
            }
        };
        // 先登記等待者再通知：通知之後馬上就可能有人回答。
        let rx = {
            let mut entries = self.entries.lock();
            let e = entries.get_mut(task_id).ok_or("這張卡已經結束。")?;
            let (tx, rx) = tokio::sync::oneshot::channel();
            e.approvals.insert(row.id, tx);
            rx
        };
        if let Some(f) = self.on_question.get() {
            f(label, &row);
        }
        let answer = rx.await;
        if !crate::tasks::questions::has_open(pool, task_id).await.unwrap_or(false) {
            self.set_asking(task_id, false);
        }
        match answer {
            Ok(a) if a.trim() == APPROVE => Ok(()),
            Ok(a) => Err(format!("使用者拒絕了這句寫入（回答：{}）。不要換個寫法硬做，改在待確認事項說明。", a.trim())),
            Err(_) => Err("核准已取消（卡片已結束）。".into()),
        }
    }

    /// 這張卡的 ask_human 要掛著等答案（Codex 卡片）。在 `register` 之後呼叫。
    pub fn set_blocking_ask(&self, task_id: &str) {
        if let Some(e) = self.entries.lock().get_mut(task_id) {
            e.blocking_ask = true;
        }
    }

    /// 把答案交給正在等的提問；有人在等就回 true（呼叫端就不必把答案打進分頁）。
    pub fn give_answer(&self, task_id: &str, question_id: i64, text: &str) -> bool {
        let tx = self.entries.lock().get_mut(task_id).and_then(|e| e.waiters.remove(&question_id));
        tx.is_some_and(|tx| tx.send(text.to_string()).is_ok())
    }

    /// 這個問題有沒有提問請求正掛著等答案。
    pub fn has_waiter(&self, task_id: &str, question_id: i64) -> bool {
        self.entries.lock().get(task_id).is_some_and(|e| e.waiters.contains_key(&question_id))
    }

    /// 這張卡的提問是不是掛著等答案（Codex 卡片）。
    pub fn is_blocking_ask(&self, task_id: &str) -> bool {
        self.entries.lock().get(task_id).is_some_and(|e| e.blocking_ask)
    }

    /// Codex 的 SessionStart hook：記下對話 id 與記錄路徑。
    async fn record_session(&self, task_id: &str, body: &serde_json::Value) {
        let Some(pool) = self.entries.lock().get(task_id).map(|e| e.pool.clone()) else { return };
        if let Some(sid) = body.get("session_id").and_then(|v| v.as_str()) {
            if let Err(e) = store::set_session_id(&pool, task_id, sid).await {
                eprintln!("codex session id {task_id}: {e}");
            }
        }
        if let Some(path) = body.get("transcript_path").and_then(|v| v.as_str()) {
            if let Err(e) = store::set_session_path(&pool, task_id, path).await {
                eprintln!("codex session path {task_id}: {e}");
            }
        }
    }

    pub fn unregister(&self, task_id: &str) {
        self.entries.lock().remove(task_id);
    }

    /// 在等額度、等人回覆或等決策：monitor 據此不判卡住、不把 bell 當完成。
    pub fn is_waiting(&self, task_id: &str) -> bool {
        self.entries.lock().get(task_id).is_some_and(|e| e.wait.is_some() || e.reply_wait || e.asking)
    }

    /// Claude 結束一輪回覆。寬限期滿時還是同一輪、沒有新動靜、卡片也還在 → 在等人回覆。
    pub fn turn_ended(self: &Arc<Self>, task_id: &str) {
        let turn = {
            let mut entries = self.entries.lock();
            let Some(e) = entries.get_mut(task_id) else { return };
            e.turn += 1;
            e.turn_pending = true;
            e.turn
        };
        let me = self.clone();
        let task_id = task_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(me.reply_grace).await;
            let still = me.entries.lock().get(&task_id).is_some_and(|e| e.turn == turn && e.turn_pending);
            if still {
                me.set_reply_wait(&task_id, true).await;
            }
        });
    }

    /// 又在工作了（或使用者回覆了）：清掉等額度與等回覆。
    pub async fn working(&self, task_id: &str) {
        if let Some(e) = self.entries.lock().get_mut(task_id) {
            e.turn_pending = false;
        }
        self.set_waiting(task_id, false).await;
        self.set_reply_wait(task_id, false).await;
    }

    async fn set_reply_wait(&self, task_id: &str, waiting: bool) {
        let (pool, label) = {
            let mut entries = self.entries.lock();
            let Some(e) = entries.get_mut(task_id) else { return };
            if e.reply_wait == waiting {
                return;
            }
            e.reply_wait = waiting;
            (e.pool.clone(), e.label.clone())
        };
        let at = waiting.then(|| chrono::Utc::now().timestamp());
        if let Err(e) = store::set_reply_wait(&pool, task_id, at).await {
            eprintln!("set_reply_wait {task_id}: {e}");
        }
        if let Some(f) = self.on_change.get() {
            f();
        }
        if waiting {
            if let Some(f) = self.on_reply_wait.get() {
                f(&label);
            }
        }
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
            "turn-end" => state.turn_ended(&task_id),
            "working" => {
                state.tool_used(&task_id, &json).await;
                state.working(&task_id).await
            }
            "codex-session" => state.record_session(&task_id, &json).await,
            _ => {}
        }
        // Claude Code 會把回應當成 hook 輸出解析；空物件 = 沒有意見。
        "{}"
    }

    /// `ask_human` 的 MCP 端點：只回 JSON（不開 SSE），見 `tasks::ask`。
    async fn mcp(
        State(state): State<Arc<QuotaState>>,
        AxPath((secret, task_id)): AxPath<(String, String)>,
        body: axum::body::Bytes,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if state.secret() != Some(secret.as_str()) {
            return axum::http::StatusCode::NOT_FOUND.into_response();
        }
        let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return axum::http::StatusCode::BAD_REQUEST.into_response();
        };
        match crate::tasks::ask::classify_with(&msg, state.has_db(&task_id)) {
            crate::tasks::ask::Rpc::Notification => axum::http::StatusCode::ACCEPTED.into_response(),
            crate::tasks::ask::Rpc::Respond(v) => axum::Json(v).into_response(),
            crate::tasks::ask::Rpc::Ask { id, question } => {
                let (text, is_error) = state.ask(&task_id, &question).await;
                axum::Json(crate::tasks::ask::tool_result(&id, &text, is_error)).into_response()
            }
            crate::tasks::ask::Rpc::Db { id, call } => {
                let (text, is_error) = state.db(&task_id, call).await;
                axum::Json(crate::tasks::ask::tool_result(&id, &text, is_error)).into_response()
            }
        }
    }

    async fn no_stream() -> axum::http::StatusCode {
        axum::http::StatusCode::METHOD_NOT_ALLOWED
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let app = axum::Router::new()
        .route("/quota/{secret}/{task_id}/{kind}", post(handle))
        .route("/mcp/{secret}/{task_id}", post(mcp).get(no_stream).delete(|| async { axum::http::StatusCode::OK }))
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

    #[test]
    fn lean_dispatch_turns_off_the_users_plugins() {
        let plugins = user_enabled_plugins(r#"{"enabledPlugins":{"superpowers@x":true,"swift-lsp@x":true,"old@x":false},"model":"opus"}"#);
        assert_eq!(plugins, vec!["superpowers@x".to_string(), "swift-lsp@x".to_string()]);
        assert!(user_enabled_plugins("not json").is_empty());
        let v: serde_json::Value = serde_json::from_str(&hook_settings_json_with("http://h", &plugins)).unwrap();
        assert_eq!(v["enabledPlugins"], serde_json::json!({"superpowers@x": false, "swift-lsp@x": false}));
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["url"], "http://h/turn-end", "hook 照舊");
        let plain: serde_json::Value = serde_json::from_str(&hook_settings_json("http://h")).unwrap();
        assert!(plain.get("enabledPlugins").is_none(), "沒開精簡啟動就不動外掛");
    }
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[test]
    fn settings_env_is_added_without_losing_hooks() {
        let s = settings_with_env(&hook_settings_json("http://x/quota/s/t"), "MCP_TOOL_TIMEOUT", APPROVAL_TOOL_TIMEOUT_MS);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["env"]["MCP_TOOL_TIMEOUT"], "86400000");
        assert!(v["hooks"]["Stop"].is_array());
    }

    #[test]
    fn hook_settings_point_every_event_at_the_task_url() {
        let v: serde_json::Value = serde_json::from_str(&hook_settings_json("http://127.0.0.1:9/quota/s/t")).unwrap();
        let url = |ev: &str| v["hooks"][ev][0]["hooks"][0]["url"].as_str().unwrap().to_string();
        assert_eq!(v["hooks"]["StopFailure"][0]["hooks"][0]["type"], "http");
        assert_eq!(url("StopFailure"), "http://127.0.0.1:9/quota/s/t/stop-failure");
        assert_eq!(url("PostToolUse"), "http://127.0.0.1:9/quota/s/t/working");
        assert_eq!(url("UserPromptSubmit"), "http://127.0.0.1:9/quota/s/t/working");
        assert_eq!(url("Stop"), "http://127.0.0.1:9/quota/s/t/turn-end");
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

    /// PostToolUse 送來的是 Read 某個技能的 SKILL.md → 記一筆技能使用；讀別的檔不算。
    #[tokio::test]
    async fn reading_a_skill_file_is_recorded_as_a_skill_use() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        let state = QuotaState::new();
        state.register(&id, "tab", pool.clone(), ReplyLabel { agent_id: Some("a1".into()), ..label() });
        state.set_custom_skills(&id, vec![("htbtw-egl".into(), std::path::PathBuf::from("/s/htbtw-egl"))]);
        let base = state.hook_base(&id).await.unwrap();
        let client = reqwest::Client::new();
        let post = |body: serde_json::Value| client.post(format!("{base}/working")).json(&body).send();

        post(serde_json::json!({"tool_name":"Read","tool_input":{"file_path":"/s/other.md"}})).await.unwrap();
        post(serde_json::json!({"tool_name":"Bash","tool_input":{"command":"cat /s/htbtw-egl/SKILL.md"}})).await.unwrap();
        assert!(crate::tasks::custom_skills::recent_uses(&pool, 10).await.unwrap().is_empty());

        post(serde_json::json!({"tool_name":"Read","tool_input":{"file_path":"/s/htbtw-egl/SKILL.md"}})).await.unwrap();
        let uses = crate::tasks::custom_skills::recent_uses(&pool, 10).await.unwrap();
        assert_eq!(uses.len(), 1);
        assert_eq!((uses[0].skill.as_str(), uses[0].agent_id.as_deref(), uses[0].task_id.as_str()), ("htbtw-egl", Some("a1"), id.as_str()));
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
        state.register(&id, "tab", pool.clone(), label());
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
        state.register("a", "tab-a", pool.clone(), label());
        state.register("b", "tab-b", pool.clone(), label());
        state.set_waiting("a", true).await;
        let far = Instant::now() + RETRY_AFTER + Duration::from_secs(1);
        let due = state.due_nudges(far, None);
        assert_eq!(due, vec![("a".to_string(), "tab-a".to_string(), Nudge::Retry)]);
        state.unregister("a");
        assert!(state.due_nudges(far, None).is_empty());
    }

    fn label() -> ReplyLabel {
        ReplyLabel { title: "規格".into(), project_name: "P".into(), ..Default::default() }
    }

    /// Claude 結束一輪回覆（`Stop`）後一陣子都沒動靜、卡片也還沒結案 → 在等人回覆。
    #[tokio::test]
    async fn a_turn_end_without_follow_up_becomes_a_reply_wait() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        let state = QuotaState::with_reply_grace(Duration::from_millis(150));
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let a = asked.clone();
        state.set_on_reply_wait(move |label| a.lock().push(label.title.clone()));
        state.register(&id, "tab", pool.clone(), label());
        let base = state.hook_base(&id).await.unwrap();
        let client = reqwest::Client::new();
        let post = |path: &str| client.post(format!("{base}/{path}")).body("{}").send();

        // 一輪結束後馬上又在動（例如接著處理排隊的訊息）：不算在等。
        post("turn-end").await.unwrap();
        post("working").await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!state.is_waiting(&id));

        // 一輪結束後安靜下來：在等人回覆。等待期間 monitor 不判卡住（is_waiting）。
        post("turn-end").await.unwrap();
        assert!(!state.is_waiting(&id), "寬限期內還不算");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(state.is_waiting(&id));
        assert!(store::get_task(&pool, &id).await.unwrap().unwrap().reply_wait_at.is_some(), "畫面要看得到");
        assert_eq!(*asked.lock(), vec!["規格".to_string()], "要通知使用者一次");

        // 使用者回覆了：解除。
        post("working").await.unwrap();
        assert!(!state.is_waiting(&id));
        assert_eq!(store::get_task(&pool, &id).await.unwrap().unwrap().reply_wait_at, None);

        // 一輪結束後卡片就結案（印了完成標記）：解除登記，不算在等。
        post("turn-end").await.unwrap();
        state.unregister(&id);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(asked.lock().len(), 1);
    }

    #[tokio::test]
    async fn finishing_a_card_clears_its_reply_wait_column() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        store::set_reply_wait(&pool, &id, Some(5)).await.unwrap();
        store::finish_task(&pool, &id, "success", None, None).await.unwrap();
        assert_eq!(store::get_task(&pool, &id).await.unwrap().unwrap().reply_wait_at, None);
    }

    /// 派出去的 claude 透過 `--mcp-config` 連到這個端點呼叫 `ask_human`：建立問題、卡片進入
    /// 「等決策」（不判卡住、完成標記先不算），通知前端。
    #[tokio::test]
    async fn the_mcp_endpoint_turns_ask_human_into_an_open_question() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&pool, &id, "tab").await;
        let state = QuotaState::new();
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let a = asked.clone();
        state.set_on_question(move |_, q| a.lock().push(q.question.clone()));
        state.register(&id, "tab", pool.clone(), label());
        let url = state.mcp_url(&id).await.unwrap();
        let client = reqwest::Client::new();
        let rpc = |body: serde_json::Value| {
            let c = client.clone();
            let url = url.clone();
            async move { c.post(url).json(&body).send().await.unwrap() }
        };

        let init = rpc(serde_json::json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25"}})).await;
        assert_eq!(init.status(), 200);
        let init: serde_json::Value = init.json().await.unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "aiterm");
        assert_eq!(rpc(serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await.status(), 202);
        assert_eq!(client.get(&url).send().await.unwrap().status(), 405, "不提供 SSE 串流");

        assert!(!state.is_waiting(&id) && !state.is_holding_marker(&id));
        let call: serde_json::Value = rpc(serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ask_human",
            "arguments":{"question":"名稱用中文？","options":["中文","英文"]}}})).await.json().await.unwrap();
        let text = call["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("結束這一輪"), "{text}");
        assert_eq!(call["result"]["isError"], false);
        let open = crate::tasks::questions::list_open(&pool).await.unwrap();
        assert_eq!(open.len(), 1);
        assert!(text.contains(&open[0].label()));
        assert!(state.is_waiting(&id), "等決策時不判卡住");
        assert!(state.is_holding_marker(&id), "等決策時完成標記先不算結案");
        assert_eq!(*asked.lock(), vec!["名稱用中文？".to_string()]);

        state.set_asking(&id, false);
        assert!(!state.is_waiting(&id) && !state.is_holding_marker(&id));

        // 密鑰不對：拒絕。卡片已經不在（結案了）：工具回錯誤，不建問題。
        let wrong = url.replace("/mcp/", "/mcp/x");
        assert_eq!(client.post(&wrong).json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).send().await.unwrap().status(), 404);
        state.unregister(&id);
        let gone: serde_json::Value = rpc(serde_json::json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ask_human",
            "arguments":{"question":"還在嗎？"}}})).await.json().await.unwrap();
        assert_eq!(gone["result"]["isError"], true);
        assert_eq!(crate::tasks::questions::list_open(&pool).await.unwrap().len(), 1);
    }

    /// 假的資料庫後端：記下執行過的 SQL，結果固定兩列。
    fn fake_db(ran: Arc<parking_lot::Mutex<Vec<String>>>) -> crate::tasks::quota::DbBackend {
        DbBackend {
            connections: Box::new(|| {
                vec![
                    DbConnInfo { id: "dev".into(), name: "Dev".into(), db_type: "sqlite".into(), database: "dev.db".into() },
                    DbConnInfo { id: "prod".into(), name: "Prod".into(), db_type: "postgres".into(), database: "p".into() },
                ]
            }),
            run: Box::new(move |call| {
                let ran = ran.clone();
                Box::pin(async move {
                    if let crate::tasks::ask::DbCall::Query { sql, .. } = &call {
                        ran.lock().push(sql.clone());
                    }
                    Ok(DbOutput { text: "[1,2]".into(), rows: Some(2) })
                })
            }),
        }
    }

    async fn db_card(mode: crate::tasks::db_skill::DbMode) -> (SqlitePool, String, Arc<QuotaState>, Arc<parking_lot::Mutex<Vec<String>>>) {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&pool, &id, "tab").await;
        let state = QuotaState::new();
        let ran = Arc::new(parking_lot::Mutex::new(Vec::new()));
        state.set_db_backend(fake_db(ran.clone()));
        state.register(&id, "tab", pool.clone(), ReplyLabel { agent_id: Some("a1".into()), ..label() });
        state.set_db_skill(&id, Some(crate::tasks::db_skill::DbSkill { connections: vec!["dev".into()], mode }));
        (pool, id, state, ran)
    }

    fn query(conn: &str, sql: &str) -> crate::tasks::ask::DbCall {
        crate::tasks::ask::DbCall::Query { connection_id: conn.into(), sql: sql.into() }
    }

    #[tokio::test]
    async fn db_calls_respect_the_skill_and_are_logged() {
        use crate::tasks::db_skill::{recent_queries, DbMode};
        let (pool, id, state, ran) = db_card(DbMode::Read).await;

        let (text, err) = state.db(&id, crate::tasks::ask::DbCall::ListConnections).await;
        assert!(!err && text.contains("Dev") && !text.contains("Prod"), "只列開放的連線：{text}");
        assert!(text.contains("唯讀"), "{text}");

        let (text, err) = state.db(&id, query("dev", "SELECT * FROM t")).await;
        assert!(!err && text.contains("[1,2]"), "{text}");
        let (text, err) = state.db(&id, query("dev", "DELETE FROM t")).await;
        assert!(err && text.contains("唯讀"), "{text}");
        let (_, err) = state.db(&id, query("prod", "SELECT 1")).await;
        assert!(err, "沒開放的連線");
        assert_eq!(*ran.lock(), vec!["SELECT * FROM t"], "被擋下的不能執行");

        let rows = recent_queries(&pool, 10).await.unwrap();
        assert_eq!(rows.len(), 3, "放行與擋下的都要記：{rows:?}");
        let ok = rows.iter().find(|r| r.sql == "SELECT * FROM t").unwrap();
        assert_eq!((ok.decision.as_str(), ok.rows, ok.connection_name.as_str(), ok.agent_id.as_deref()), ("allowed", Some(2), "Dev", Some("a1")));
        assert!(ok.finished_at.is_some());
        assert_eq!(rows.iter().filter(|r| r.decision == "denied").count(), 2);

        // 沒裝技能的卡：一律拒絕。
        state.set_db_skill(&id, None);
        assert!(state.db(&id, query("dev", "SELECT 1")).await.1);
    }

    #[tokio::test]
    async fn a_write_waits_for_approval_and_runs_only_if_approved() {
        use crate::tasks::db_skill::{recent_queries, DbMode};
        let (pool, id, state, ran) = db_card(DbMode::Approve).await;
        let next_question = |pool: SqlitePool| async move {
            loop {
                if let Some(q) = crate::tasks::questions::list_open(&pool).await.unwrap().pop() {
                    break q;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };

        let s = state.clone();
        let i = id.clone();
        let pending = tokio::spawn(async move { s.db(&i, query("dev", "UPDATE t SET a = 1")).await });
        let q = next_question(pool.clone()).await;
        assert!(q.context.as_deref().unwrap_or("").contains("UPDATE t SET a = 1"), "核准要看得到完整 SQL");
        assert_eq!(serde_json::from_str::<Vec<String>>(&q.options).unwrap(), vec![APPROVE.to_string(), REJECT.to_string()]);
        assert!(state.is_waiting(&id), "等核准時不判卡住");
        while !state.has_approval(&id, q.id) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(ran.lock().is_empty(), "還沒核准不能執行");
        let waiting = recent_queries(&pool, 10).await.unwrap();
        assert_eq!(waiting.len(), 1, "等核准時就要有一筆，畫面才顯示得出「等你核准」");
        assert_eq!((waiting[0].decision.as_str(), waiting[0].finished_at), ("pending", None));
        // 正式流程：指令先把答案存進資料庫，再交給等待者。
        crate::tasks::questions::answer(&pool, q.id, APPROVE, "app").await.unwrap();
        assert!(state.give_approval(&id, q.id, APPROVE));
        let (text, err) = pending.await.unwrap();
        assert!(!err, "{text}");
        assert_eq!(*ran.lock(), vec!["UPDATE t SET a = 1"]);
        assert!(!state.is_waiting(&id));

        let s = state.clone();
        let i = id.clone();
        let pending = tokio::spawn(async move { s.db(&i, query("dev", "DELETE FROM t")).await });
        let q = loop {
            let q = next_question(pool.clone()).await;
            if q.context.as_deref().unwrap_or("").contains("DELETE") {
                break q;
            }
        };
        while !state.has_approval(&id, q.id) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        crate::tasks::questions::answer(&pool, q.id, REJECT, "app").await.unwrap();
        assert!(state.give_approval(&id, q.id, REJECT));
        let (text, err) = pending.await.unwrap();
        assert!(err && text.contains("拒絕"), "{text}");
        assert_eq!(ran.lock().len(), 1, "拒絕的不執行");

        let rows = recent_queries(&pool, 10).await.unwrap();
        assert_eq!(rows.len(), 2, "等待那筆就是後來執行的那筆，不另外多記：{rows:?}");
        assert_eq!(rows.iter().find(|r| r.sql.starts_with("UPDATE")).unwrap().decision, "approved");
        assert!(rows.iter().all(|r| r.finished_at.is_some()));
        assert_eq!(rows.iter().find(|r| r.sql.starts_with("DELETE")).unwrap().decision, "denied");
    }

    /// Codex 卡片的 ask_human 掛著等答案，答案直接當工具結果回去（不打字進 TUI：小模型會先自己猜答案）。
    #[tokio::test]
    async fn a_codex_card_waits_on_ask_human_until_the_answer_arrives() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&pool, &id, "tab").await;
        let state = QuotaState::new();
        state.register(&id, "tab", pool.clone(), label());
        state.set_blocking_ask(&id);
        let url = state.mcp_url(&id).await.unwrap();
        let call = |qid: i64, q: &str| {
            let url = url.clone();
            let body = serde_json::json!({"jsonrpc":"2.0","id":qid,"method":"tools/call","params":{"name":"ask_human","arguments":{"question":q}}});
            tokio::spawn(async move { reqwest::Client::new().post(url).json(&body).send().await.unwrap().json::<serde_json::Value>().await.unwrap() })
        };

        let pending = call(1, "顏色？");
        let q = loop {
            if let Some(q) = crate::tasks::questions::list_open(&pool).await.unwrap().pop() {
                break q;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(state.is_waiting(&id), "等答案時不判卡住");
        assert!(!pending.is_finished(), "還沒回答就不能回傳");
        // 等待者在問題寫進資料庫之後才登記，輪詢看到問題時可能還差一點。
        while !state.has_waiter(&id, q.id) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(state.is_blocking_ask(&id));
        assert!(!state.has_waiter(&id, q.id + 99));
        assert!(!state.give_answer(&id, q.id + 99, "x"), "別的問題沒人在等");
        assert!(state.give_answer(&id, q.id, "Q1 的回答：藍色"));
        let res = pending.await.unwrap();
        assert_eq!(res["result"]["content"][0]["text"], "Q1 的回答：藍色");
        assert_eq!(res["result"]["isError"], false);

        // 卡片結束：還在等的提問收到「已取消」。
        let pending = call(2, "還要嗎？");
        while crate::tasks::questions::list_open(&pool).await.unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        state.unregister(&id);
        let res = pending.await.unwrap();
        assert_eq!(res["result"]["isError"], true);
        assert!(res["result"]["content"][0]["text"].as_str().unwrap().contains("取消"));
    }

    /// Codex 的 SessionStart hook 回報對話 id 與記錄路徑，寫進卡片（接著做、token 統計用）。
    #[tokio::test]
    async fn the_codex_session_hook_records_the_session_on_the_card() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        let state = QuotaState::new();
        state.register(&id, "tab", pool.clone(), label());
        let base = state.hook_base(&id).await.unwrap();
        reqwest::Client::new()
            .post(format!("{base}/codex-session"))
            .body(r#"{"session_id":"01a1-s","transcript_path":"/h/.codex/sessions/rollout-x-01a1-s.jsonl","hook_event_name":"SessionStart"}"#)
            .send()
            .await
            .unwrap();
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.session_id.as_deref(), Some("01a1-s"));
        assert_eq!(row.session_path.as_deref(), Some("/h/.codex/sessions/rollout-x-01a1-s.jsonl"));
    }
}
