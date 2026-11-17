//! 系統連線的定時任務：到點用 Claude Code 無人模式讓 AI 在某個系統上完成一段目標。
//! 見 docs/superpowers/specs/2026-11-17-system-jobs-design.md。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDateTime;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, SqlitePool};

use super::endpoint::MCP_SERVER_ID;
use super::state::SystemsState;
use super::types::{Mode, SystemConnection};
use crate::tasks::schedules::{self, Spec};

pub const MAX_RUNS_KEPT: i64 = 20;
pub const MAX_FAIL_STREAK: i64 = 3;
pub const RUN_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const SUMMARY_LIMIT: usize = 4000;
pub const REASON_FAILURES: &str = "連續失敗";

/// 同一個系統一次只做一件事用的鎖名稱。
pub fn system_key(system_id: &str) -> String {
    format!("system:{system_id}")
}

// ── 純函式 ─────────────────────────────────────────────────────────────────

/// 給 Claude 的提示：在哪個系統、要做什麼，以及做事的規矩。
pub fn build_prompt(system_name: &str, system_id: &str, goal: &str) -> String {
    format!(
        "你是自動執行的助理，現在要在外部系統「{system_name}」（system id：{system_id}）上完成下面的目標。\n\n\
         目標：\n{goal}\n\n\
         規矩：\n\
         1. 只使用 sys_* 工具，每次呼叫都要帶 system=\"{system_id}\"。\n\
         2. 先呼叫 sys_recipe_list 看有沒有適用的配方，有就用 sys_recipe_run，不要重新摸索。\n\
         3. 每個動作都要帶 intent，並以「讀：」或「寫：」開頭說明要做什麼。\n\
         4. 動作被閘門拒絕（回覆含「被閘門拒絕」）時不可重試；停下來，回報需要開放什麼權限。\n\
         5. 登入失效、找不到元素、頁面不是預期的樣子時，不要亂試；停下來回報。\n\
         6. 沒有人會回答你的問題。完成後用一兩句話回報結果；做不到就說明卡在哪裡。\n\
         7. 回報的最後一行必須是「結果：成功」或「結果：失敗：原因」，讓程式知道事情有沒有做成。"
    )
}

/// `--mcp-config` 的內容：指向本機的系統連線端點。
pub fn mcp_config_json(port: u16, token: &str) -> String {
    json!({ "mcpServers": { MCP_SERVER_ID: {
        "type": "http",
        "url": format!("http://127.0.0.1:{port}/systems/mcp"),
        "headers": { "Authorization": format!("Bearer {token}") }
    } } })
    .to_string()
}

/// 無人模式的引數（提示走標準輸入，免得多行中文在各平台的命令列引號出問題）。
pub fn claude_args(mcp_path: &str) -> Vec<String> {
    vec![
        "-p".into(),
        "--mcp-config".into(),
        mcp_path.into(),
        "--allowedTools".into(),
        format!("mcp__{MCP_SERVER_ID}"),
        "--output-format".into(),
        "text".into(),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Run,
    Skip(&'static str),
}

/// 該不該在這個時間點跑：任務停用或系統停用或上一輪還在跑都跳過。
pub fn should_run(job_enabled: bool, system_enabled: bool, already_running: bool) -> Decision {
    if !job_enabled {
        return Decision::Skip("任務已停用");
    }
    if !system_enabled {
        return Decision::Skip("系統連線已停用");
    }
    if already_running {
        return Decision::Skip("上一輪還在執行");
    }
    Decision::Run
}

/// 這個系統的權限模式適不適合無人執行（只讀、逐次核准會讓寫入卡住）。
pub fn unattended_warning(mode: Mode) -> Option<&'static str> {
    match mode {
        Mode::ReadOnly => Some("這個系統是只讀，任務遇到寫入會被拒絕"),
        Mode::ApproveEach => Some("這個系統要逐次核准，沒人在時寫入會等 5 分鐘後被拒絕"),
        Mode::Rules | Mode::Autonomous => None,
    }
}

/// 看 AI 回報的最後一個「結果：…」行：成功回 `Some(true)`、失敗回 `Some(false)`，沒寫就是 `None`。
pub fn final_verdict(summary: &str) -> Option<bool> {
    let line = summary.lines().rev().map(str::trim).find(|l| l.starts_with("結果：") || l.starts_with("結果:"))?;
    let rest = line.trim_start_matches("結果：").trim_start_matches("結果:").trim();
    if rest.starts_with("成功") {
        Some(true)
    } else if rest.starts_with("失敗") {
        Some(false)
    } else {
        None
    }
}

/// 回覆給寄件者的內容不需要那行給程式看的「結果：…」：拿掉最後一個結果行（沒有就原樣）。
pub fn strip_verdict_line(summary: &str) -> String {
    let lines: Vec<&str> = summary.trim_end().lines().collect();
    match lines.iter().rposition(|l| {
        let t = l.trim();
        t.starts_with("結果：") || t.starts_with("結果:")
    }) {
        Some(i) if lines[i + 1..].iter().all(|l| l.trim().is_empty()) => lines[..i].join("\n").trim_end().to_string(),
        _ => summary.trim_end().to_string(),
    }
}

fn tail_chars(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.trim().chars().collect();
    chars[chars.len().saturating_sub(n)..].iter().collect()
}

// ── 儲存 ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Job {
    pub id: String,
    pub system_id: String,
    pub name: String,
    pub goal: String,
    pub kind: String,
    pub spec_json: String,
    pub enabled: bool,
    pub created_ms: i64,
    pub last_fired_ms: i64,
    pub fail_streak: i64,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct JobRun {
    pub id: i64,
    pub job_id: String,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
    pub status: String,
    pub summary: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewJob {
    pub system_id: String,
    pub name: String,
    pub goal: String,
    pub kind: String,
    pub spec: Spec,
}

pub async fn init(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS system_jobs (
            id TEXT PRIMARY KEY NOT NULL,
            system_id TEXT NOT NULL,
            name TEXT NOT NULL,
            goal TEXT NOT NULL,
            kind TEXT NOT NULL,
            spec_json TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_ms INTEGER NOT NULL,
            last_fired_ms INTEGER NOT NULL,
            fail_streak INTEGER NOT NULL DEFAULT 0,
            disabled_reason TEXT
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS system_job_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL,
            started_ms INTEGER NOT NULL,
            finished_ms INTEGER,
            status TEXT NOT NULL,
            summary TEXT NOT NULL DEFAULT ''
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn validate_new(j: &NewJob) -> Result<(), String> {
    if j.name.trim().is_empty() {
        return Err("任務名稱不可空白".into());
    }
    if j.goal.trim().is_empty() {
        return Err("目標不可空白".into());
    }
    schedules::validate(&j.kind, &j.spec)
}

pub async fn create(pool: &SqlitePool, j: &NewJob) -> Result<String, String> {
    validate_new(j)?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO system_jobs (id, system_id, name, goal, kind, spec_json, enabled, created_ms, last_fired_ms)
         VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?)",
    )
    .bind(&id)
    .bind(&j.system_id)
    .bind(j.name.trim())
    .bind(j.goal.trim())
    .bind(&j.kind)
    .bind(serde_json::to_string(&j.spec).map_err(|e| e.to_string())?)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub async fn update(pool: &SqlitePool, id: &str, j: &NewJob) -> Result<(), String> {
    validate_new(j)?;
    let r = sqlx::query("UPDATE system_jobs SET name = ?, goal = ?, kind = ?, spec_json = ?, last_fired_ms = ? WHERE id = ?")
        .bind(j.name.trim())
        .bind(j.goal.trim())
        .bind(&j.kind)
        .bind(serde_json::to_string(&j.spec).map_err(|e| e.to_string())?)
        .bind(now_ms()) // 改了週期就從現在起算，不補跑
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    if r.rows_affected() == 0 {
        return Err("找不到這個任務".into());
    }
    Ok(())
}

pub async fn list(pool: &SqlitePool, system_id: Option<&str>) -> Result<Vec<Job>, sqlx::Error> {
    match system_id {
        Some(s) => sqlx::query_as("SELECT * FROM system_jobs WHERE system_id = ? ORDER BY created_ms").bind(s).fetch_all(pool).await,
        None => sqlx::query_as("SELECT * FROM system_jobs ORDER BY created_ms").fetch_all(pool).await,
    }
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<Job>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM system_jobs WHERE id = ?").bind(id).fetch_optional(pool).await
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM system_job_runs WHERE job_id = ?").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM system_jobs WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

/// 啟用／停用。重新啟用時清掉失敗記錄並從現在起算（不補跑停用期間的時間）。
pub async fn set_enabled(pool: &SqlitePool, id: &str, enabled: bool) -> Result<(), sqlx::Error> {
    if enabled {
        sqlx::query("UPDATE system_jobs SET enabled = 1, fail_streak = 0, disabled_reason = NULL, last_fired_ms = ? WHERE id = ?")
            .bind(now_ms())
            .bind(id)
            .execute(pool)
            .await?;
    } else {
        sqlx::query("UPDATE system_jobs SET enabled = 0, disabled_reason = NULL WHERE id = ?").bind(id).execute(pool).await?;
    }
    Ok(())
}

pub async fn mark_fired(pool: &SqlitePool, id: &str, at_ms: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE system_jobs SET last_fired_ms = ? WHERE id = ?").bind(at_ms).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn start_run(pool: &SqlitePool, job_id: &str, started_ms: i64) -> Result<i64, sqlx::Error> {
    let r = sqlx::query("INSERT INTO system_job_runs (job_id, started_ms, status) VALUES (?, ?, 'running')")
        .bind(job_id)
        .bind(started_ms)
        .execute(pool)
        .await?;
    Ok(r.last_insert_rowid())
}

pub async fn record_skip(pool: &SqlitePool, job_id: &str, why: &str) -> Result<(), sqlx::Error> {
    let now = now_ms();
    sqlx::query("INSERT INTO system_job_runs (job_id, started_ms, finished_ms, status, summary) VALUES (?, ?, ?, 'skipped', ?)")
        .bind(job_id)
        .bind(now)
        .bind(now)
        .bind(why)
        .execute(pool)
        .await?;
    prune_runs(pool, job_id).await
}

async fn prune_runs(pool: &SqlitePool, job_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM system_job_runs WHERE job_id = ? AND id NOT IN
         (SELECT id FROM system_job_runs WHERE job_id = ? ORDER BY id DESC LIMIT ?)",
    )
    .bind(job_id)
    .bind(job_id)
    .bind(MAX_RUNS_KEPT)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_runs(pool: &SqlitePool, job_id: &str) -> Result<Vec<JobRun>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM system_job_runs WHERE job_id = ? ORDER BY id DESC").bind(job_id).fetch_all(pool).await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub status: &'static str, // ok | failed | timeout
    pub summary: String,
}

/// 結案：寫入執行記錄、更新連續失敗次數。回傳這次是否因連續失敗而被自動停用。
pub async fn settle(pool: &SqlitePool, job_id: &str, run_id: i64, o: &Outcome) -> Result<bool, sqlx::Error> {
    sqlx::query("UPDATE system_job_runs SET finished_ms = ?, status = ?, summary = ? WHERE id = ?")
        .bind(now_ms())
        .bind(o.status)
        .bind(tail_chars(&o.summary, SUMMARY_LIMIT))
        .bind(run_id)
        .execute(pool)
        .await?;
    prune_runs(pool, job_id).await?;
    if o.status == "ok" {
        sqlx::query("UPDATE system_jobs SET fail_streak = 0 WHERE id = ?").bind(job_id).execute(pool).await?;
        return Ok(false);
    }
    sqlx::query("UPDATE system_jobs SET fail_streak = fail_streak + 1 WHERE id = ?").bind(job_id).execute(pool).await?;
    let streak: i64 = sqlx::query_scalar("SELECT fail_streak FROM system_jobs WHERE id = ?").bind(job_id).fetch_one(pool).await?;
    if streak >= MAX_FAIL_STREAK {
        sqlx::query("UPDATE system_jobs SET enabled = 0, disabled_reason = ? WHERE id = ?").bind(REASON_FAILURES).bind(job_id).execute(pool).await?;
        return Ok(true);
    }
    Ok(false)
}

// ── 執行 claude ────────────────────────────────────────────────────────────

/// 執行一次無人模式的 claude：提示走標準輸入，stdout 是結果。非 0 結束碼＝失敗，逾時就終止。
pub async fn run_claude(command: &str, mcp_json: &str, prompt: &str, work_dir: &std::path::Path, timeout: Duration) -> Outcome {
    use tokio::io::AsyncWriteExt;
    crate::tasks::path_refresh::refresh().await;
    if let Err(e) = std::fs::create_dir_all(work_dir) {
        return Outcome { status: "failed", summary: format!("無法建立工作資料夾：{e}") };
    }
    let mcp_path = work_dir.join(format!("{}.mcp.json", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::write(&mcp_path, mcp_json) {
        return Outcome { status: "failed", summary: format!("無法寫入 MCP 設定：{e}") };
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&mcp_path, std::fs::Permissions::from_mode(0o600));
    }
    let result = async {
        let mut parts = shell_words::split(command).map_err(|e| format!("claude 指令格式不正確：{e}"))?;
        if parts.is_empty() {
            parts.push("claude".into());
        }
        let program = parts.remove(0);
        parts.extend(claude_args(&mcp_path.to_string_lossy()));
        let mut cmd = crate::mcp::transport::build_command(&program, &parts, &Default::default());
        cmd.current_dir(work_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| format!("無法啟動 claude：{e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            drop(stdin);
        }
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                if out.status.success() {
                    Ok(Outcome { status: "ok", summary: stdout })
                } else {
                    let why = if stderr.trim().is_empty() { stdout } else { stderr };
                    Ok(Outcome { status: "failed", summary: format!("結束碼 {:?}：{}", out.status.code(), tail_chars(&why, 1500)) })
                }
            }
            Ok(Err(e)) => Err(format!("等待 claude 時出錯：{e}")),
            Err(_) => Ok(Outcome { status: "timeout", summary: format!("執行超過 {} 分鐘，已終止", timeout.as_secs() / 60) }),
        }
    }
    .await;
    let _ = std::fs::remove_file(&mcp_path);
    result.unwrap_or_else(|e| Outcome { status: "failed", summary: e })
}

// ── 排程 ───────────────────────────────────────────────────────────────────

pub struct JobsCtx {
    pub systems: Arc<SystemsState>,
    pub config: Arc<crate::config::ConfigStore>,
    /// 取得端點目前的 (埠, token)；端點沒在跑就是 `None`。
    pub endpoint: Arc<dyn Fn() -> Option<(u16, String)> + Send + Sync>,
    pub emit: super::state::Emitter,
    pub work_dir: std::path::PathBuf,
    pub timeout: Duration,
}

#[derive(Default)]
pub struct JobsRuntime {
    running: Mutex<HashSet<String>>,
}

impl JobsRuntime {
    pub fn is_running(&self, id: &str) -> bool {
        self.running.lock().contains(id)
    }

    /// 掃一遍：到點的任務就啟動（各自在背景執行）。回傳這次啟動了幾個。
    pub async fn tick(self: &Arc<Self>, ctx: &Arc<JobsCtx>, now: NaiveDateTime) -> usize {
        let Some(pool) = ctx.systems.audit_pool().await else { return 0 };
        let Ok(jobs) = list(&pool, None).await else { return 0 };
        let mut started = 0;
        for job in jobs.into_iter().filter(|j| j.enabled) {
            let spec: Spec = serde_json::from_str(&job.spec_json).unwrap_or_default();
            if !schedules::due(&job.kind, &spec, schedules::from_ts(job.last_fired_ms / 1000), now) {
                continue;
            }
            let system_on = ctx.systems.connection(&job.system_id).map(|c| c.enabled).unwrap_or(false);
            let _ = mark_fired(&pool, &job.id, schedules::local_ts(now) * 1000).await;
            let busy = self.is_running(&job.id) || self.is_running(&system_key(&job.system_id));
            match should_run(true, system_on, busy) {
                Decision::Skip(why) => {
                    let _ = record_skip(&pool, &job.id, why).await;
                }
                Decision::Run => {
                    started += 1;
                    let (me, ctx2, id) = (self.clone(), ctx.clone(), job.id.clone());
                    tokio::spawn(async move {
                        me.run_job(&ctx2, &id).await;
                    });
                }
            }
        }
        started
    }

    /// 執行一個任務（排程與「現在執行」共用）。回傳這次的結果；任務不存在或正在跑回 `Err`。
    pub async fn run_job(self: &Arc<Self>, ctx: &Arc<JobsCtx>, id: &str) -> Result<Outcome, String> {
        let pool = ctx.systems.audit_pool().await.ok_or("systems.db 不可用")?;
        let job = get(&pool, id).await.map_err(|e| e.to_string())?.ok_or("找不到這個任務")?;
        if !self.try_lock(id) {
            return Err("這個任務正在執行".into());
        }
        let key = system_key(&job.system_id);
        if !self.try_lock(&key) {
            self.unlock(id);
            return Err("這個系統正在處理別的事".into());
        }
        let outcome = self.run_inner(ctx, &pool, &job).await;
        self.unlock(&key);
        self.unlock(id);
        outcome
    }

    /// 同一個系統一次只做一件事（定時任務與問題收集共用）。拿到鎖回 `true`。
    pub fn try_lock(&self, key: &str) -> bool {
        self.running.lock().insert(key.to_string())
    }

    pub fn unlock(&self, key: &str) {
        self.running.lock().remove(key);
    }

    async fn run_inner(&self, ctx: &Arc<JobsCtx>, pool: &SqlitePool, job: &Job) -> Result<Outcome, String> {
        let started_ms = now_ms();
        let run_id = start_run(pool, &job.id, started_ms).await.map_err(|e| e.to_string())?;
        let outcome = self.run_goal(ctx, pool, &job.system_id, &job.goal).await;
        let disabled = settle(pool, &job.id, run_id, &outcome).await.map_err(|e| e.to_string())?;
        (ctx.emit)("systems://job-finished", json!({ "job_id": job.id, "system_id": job.system_id, "status": outcome.status }));
        if disabled {
            (ctx.emit)("systems://job-disabled", json!({ "job_id": job.id, "name": job.name, "reason": REASON_FAILURES }));
        }
        Ok(outcome)
    }

    /// 讓 AI 在某個系統上完成一段目標（定時任務與問題收集共用）：組提示、跑 claude，
    /// 並把「執行期間有寫入被拒絕或逾時」判成失敗（claude 自己結束碼是 0 也一樣）。
    pub async fn run_goal(&self, ctx: &Arc<JobsCtx>, pool: &SqlitePool, system_id: &str, goal: &str) -> Outcome {
        let started_ms = now_ms();
        let conn: SystemConnection = match ctx.systems.connection(system_id) {
            Ok(c) => c,
            Err(e) => return Outcome { status: "failed", summary: e },
        };
        let outcome = match (conn.enabled, (ctx.endpoint)()) {
            (false, _) => Outcome { status: "failed", summary: "系統連線已停用".into() },
            (_, None) => Outcome { status: "failed", summary: "系統連線端點沒有在執行".into() },
            (true, Some((port, token))) => {
                let prompt = build_prompt(&conn.name, &conn.id, goal);
                let command = ctx.config.get().task_board.claude_command;
                run_claude(&command, &mcp_config_json(port, &token), &prompt, &ctx.work_dir, ctx.timeout).await
            }
        };
        // AI 自己回報做不成（結果：失敗）也算失敗，不然「claude 結束碼 0」會把它記成成功。
        let outcome = if outcome.status == "ok" && final_verdict(&outcome.summary) == Some(false) {
            Outcome { status: "failed", summary: format!("AI 回報這件事沒有做成：\n{}", outcome.summary) }
        } else {
            outcome
        };
        match super::audit::count_blocked_since(pool, system_id, started_ms).await {
            Ok(n) if n > 0 && outcome.status == "ok" => Outcome {
                status: "failed",
                summary: format!("執行期間有 {n} 次寫入被拒絕或等核准逾時（這個系統的權限不允許無人執行這件事）。AI 的回報：\n{}", outcome.summary),
            },
            _ => outcome,
        }
    }
}

/// 一次性執行的結果：AI 的回報、做了哪些步驟，以及能不能存成配方（成功且有步驟）。
#[derive(Debug, Clone, Serialize)]
pub struct AdhocReport {
    pub status: &'static str,
    /// AI 的回報（已拿掉給程式看的「結果：」行）；失敗時是原因。
    pub summary: String,
    pub steps: Vec<StepView>,
    /// 系統猜到的參數候選，給存配方的畫面當預設。
    pub params: Vec<super::recipes::Param>,
    pub can_save: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepView {
    pub tool: String,
    pub intent: String,
    /// 這一步的重點（欄位名稱與輸入的值），給畫面顯示。
    pub detail: String,
    pub write: bool,
}

fn step_view(s: &super::recipes::Step) -> StepView {
    let mut bits: Vec<String> = Vec::new();
    if let Some(n) = s.args["target"]["$locator"]["name"].as_str().filter(|n| !n.is_empty()) {
        bits.push(n.to_string());
    }
    for k in ["text", "url", "key"] {
        if let Some(v) = s.args[k].as_str().filter(|v| !v.is_empty()) {
            bits.push(v.chars().take(80).collect());
        }
    }
    StepView { tool: s.tool.clone(), intent: s.intent.clone(), detail: bits.join(" · "), write: s.intent.trim_start().starts_with('寫') }
}

impl JobsRuntime {
    /// 「請 AI 操作」：跑一次目標，回傳結果與這次錄到的步驟。同一個系統一次只做一件事。
    /// 成功且有步驟時日誌保留（等使用者決定要不要存成配方）；其他情況清空。
    pub async fn run_adhoc(&self, ctx: &Arc<JobsCtx>, system_id: &str, goal: &str) -> Result<AdhocReport, String> {
        if goal.trim().is_empty() {
            return Err("請先寫下要 AI 做什麼".into());
        }
        let pool = ctx.systems.audit_pool().await.ok_or("systems.db 不可用")?;
        let key = system_key(system_id);
        if !self.try_lock(&key) {
            return Err("這個系統正在處理別的事".into());
        }
        ctx.systems.journal_clear(system_id).await;
        let outcome = self.run_goal(ctx, &pool, system_id, goal).await;
        let steps = ctx.systems.journal_steps(system_id).await;
        self.unlock(&key);
        let ok = outcome.status == "ok";
        let can_save = ok && !steps.is_empty();
        if !can_save {
            ctx.systems.journal_clear(system_id).await;
        }
        Ok(AdhocReport {
            status: outcome.status,
            summary: if ok { strip_verdict_line(&outcome.summary) } else { outcome.summary },
            steps: steps.iter().map(step_view).collect(),
            params: if can_save { super::recipes::suggest_params(&steps) } else { Vec::new() },
            can_save,
        })
    }

    /// 配方的「執行」：與其他操作共用同一把系統鎖。
    pub async fn run_recipe_locked(
        &self,
        ctx: &Arc<JobsCtx>,
        system_id: &str,
        name: &str,
        values: std::collections::HashMap<String, String>,
    ) -> Result<String, String> {
        let key = system_key(system_id);
        if !self.try_lock(&key) {
            return Err("這個系統正在處理別的事".into());
        }
        let r = ctx.systems.recipe_run(system_id, name, values).await;
        self.unlock(&key);
        r
    }
}

/// 啟動背景排程：每 30 秒掃一次。
pub fn spawn(rt: Arc<JobsRuntime>, ctx: Arc<JobsCtx>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            rt.tick(&ctx, chrono::Local::now().naive_local()).await;
        }
    });
}

#[allow(dead_code)]
fn _assert_value_used(_: Value) {}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let p = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init(&p).await.unwrap();
        p
    }

    fn new_job(system: &str) -> NewJob {
        NewJob {
            system_id: system.into(),
            name: "每日匯出".into(),
            goal: "匯出昨天的待處理訂單".into(),
            kind: "daily".into(),
            spec: Spec { times: vec!["09:00".into()], ..Default::default() },
        }
    }

    #[test]
    fn the_prompt_names_the_system_the_goal_and_the_rules() {
        let p = build_prompt("訂單系統", "sys-1", "匯出昨天的訂單");
        assert!(p.contains("訂單系統") && p.contains("sys-1") && p.contains("匯出昨天的訂單"));
        assert!(p.contains("sys_recipe_list") && p.contains("被閘門拒絕") && p.contains("intent"));
    }

    #[test]
    fn the_ais_own_verdict_line_decides_success_or_failure() {
        assert_eq!(final_verdict("做完了\n結果：成功"), Some(true));
        assert_eq!(final_verdict("卡住了\n結果：失敗：找不到單號欄位\n"), Some(false));
        assert_eq!(final_verdict("結果:成功"), Some(true), "半形冒號也算");
        assert_eq!(final_verdict("先寫了結果：成功\n後來又改\n結果：失敗：改了"), Some(false), "以最後一個為準");
        assert_eq!(final_verdict("只是一般回報，沒寫結果行"), None);
        assert_eq!(final_verdict("這一行含有結果：成功 但不是行首"), None);
        assert!(build_prompt("s", "id", "g").contains("結果：成功"), "提示要要求 AI 寫結果行");
    }

    #[test]
    fn the_verdict_line_is_removed_from_replies_only_when_it_is_the_last_line() {
        assert_eq!(strip_verdict_line("單號 C-5 已送出。\n\n結果：成功\n"), "單號 C-5 已送出。");
        assert_eq!(strip_verdict_line("結果：成功"), "");
        assert_eq!(strip_verdict_line("沒有結果行的回報"), "沒有結果行的回報");
        assert_eq!(strip_verdict_line("結果：成功\n後面還有內容"), "結果：成功\n後面還有內容", "不是最後一行就不動");
    }

    #[test]
    fn claude_runs_headless_with_only_the_systems_tools_allowed() {
        let a = claude_args("/tmp/x.json");
        assert_eq!(a[0], "-p");
        let i = a.iter().position(|x| x == "--allowedTools").unwrap();
        assert_eq!(a[i + 1], "mcp__aiterm-systems", "只放行這個伺服器的工具");
        assert_eq!(a[a.iter().position(|x| x == "--mcp-config").unwrap() + 1], "/tmp/x.json");
        let j: Value = serde_json::from_str(&mcp_config_json(48320, "tok")).unwrap();
        assert_eq!(j["mcpServers"]["aiterm-systems"]["url"], "http://127.0.0.1:48320/systems/mcp");
        assert_eq!(j["mcpServers"]["aiterm-systems"]["headers"]["Authorization"], "Bearer tok");
    }

    #[test]
    fn a_job_is_skipped_when_disabled_or_its_system_is_off_or_it_is_still_running() {
        assert_eq!(should_run(true, true, false), Decision::Run);
        assert!(matches!(should_run(false, true, false), Decision::Skip(_)));
        assert!(matches!(should_run(true, false, false), Decision::Skip(_)));
        assert!(matches!(should_run(true, true, true), Decision::Skip("上一輪還在執行")));
    }

    #[test]
    fn only_read_only_and_approve_each_warn_about_unattended_runs() {
        assert!(unattended_warning(Mode::ReadOnly).is_some());
        assert!(unattended_warning(Mode::ApproveEach).is_some());
        assert!(unattended_warning(Mode::Rules).is_none() && unattended_warning(Mode::Autonomous).is_none());
    }

    #[tokio::test]
    async fn jobs_are_validated_stored_listed_updated_and_deleted() {
        let p = pool().await;
        let mut bad = new_job("s1");
        bad.goal = "  ".into();
        assert!(create(&p, &bad).await.is_err());
        let mut bad = new_job("s1");
        bad.spec = Spec::default();
        assert!(create(&p, &bad).await.is_err(), "沒有時間點的每日任務不合法");

        let id = create(&p, &new_job("s1")).await.unwrap();
        create(&p, &new_job("s2")).await.unwrap();
        assert_eq!(list(&p, Some("s1")).await.unwrap().len(), 1);
        assert_eq!(list(&p, None).await.unwrap().len(), 2);

        let mut upd = new_job("s1");
        upd.name = "改名".into();
        update(&p, &id, &upd).await.unwrap();
        assert_eq!(get(&p, &id).await.unwrap().unwrap().name, "改名");
        assert!(update(&p, "ghost", &upd).await.is_err());

        run_ids(&p, &id, 2).await;
        delete(&p, &id).await.unwrap();
        assert!(get(&p, &id).await.unwrap().is_none());
        assert!(list_runs(&p, &id).await.unwrap().is_empty(), "刪任務連執行記錄一起刪");
    }

    async fn run_ids(p: &SqlitePool, id: &str, n: usize) -> Vec<i64> {
        let mut v = Vec::new();
        for i in 0..n {
            v.push(start_run(p, id, i as i64).await.unwrap());
        }
        v
    }

    fn failed() -> Outcome {
        Outcome { status: "failed", summary: "壞了".into() }
    }

    #[tokio::test]
    async fn three_failures_in_a_row_disable_the_job_and_a_success_resets_the_count() {
        let p = pool().await;
        let id = create(&p, &new_job("s1")).await.unwrap();
        let ids = run_ids(&p, &id, 6).await;
        assert!(!settle(&p, &id, ids[0], &failed()).await.unwrap());
        assert!(!settle(&p, &id, ids[1], &failed()).await.unwrap());
        assert!(!settle(&p, &id, ids[2], &Outcome { status: "ok", summary: "好".into() }).await.unwrap(), "成功歸零");
        assert_eq!(get(&p, &id).await.unwrap().unwrap().fail_streak, 0);
        assert!(!settle(&p, &id, ids[3], &failed()).await.unwrap());
        assert!(!settle(&p, &id, ids[4], &failed()).await.unwrap());
        assert!(settle(&p, &id, ids[5], &failed()).await.unwrap(), "第三次連續失敗才停用");
        let j = get(&p, &id).await.unwrap().unwrap();
        assert!(!j.enabled);
        assert_eq!(j.disabled_reason.as_deref(), Some(REASON_FAILURES));

        set_enabled(&p, &id, true).await.unwrap();
        let j = get(&p, &id).await.unwrap().unwrap();
        assert!(j.enabled && j.fail_streak == 0 && j.disabled_reason.is_none(), "重新啟用要清掉失敗記錄");
    }

    #[tokio::test]
    async fn only_the_latest_runs_are_kept_and_summaries_are_capped() {
        let p = pool().await;
        let id = create(&p, &new_job("s1")).await.unwrap();
        for i in 0..(MAX_RUNS_KEPT + 5) {
            let r = start_run(&p, &id, i).await.unwrap();
            settle(&p, &id, r, &Outcome { status: "ok", summary: "x".repeat(6000) }).await.unwrap();
        }
        let runs = list_runs(&p, &id).await.unwrap();
        assert_eq!(runs.len() as i64, MAX_RUNS_KEPT);
        assert_eq!(runs[0].summary.chars().count(), SUMMARY_LIMIT);
        record_skip(&p, &id, "上一輪還在執行").await.unwrap();
        assert_eq!(list_runs(&p, &id).await.unwrap()[0].status, "skipped");
    }

    /// 假 claude：把引數與標準輸入寫進檔案，印出一行結果。
    #[cfg(unix)]
    fn fake_claude(dir: &std::path::Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-claude.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_runner_passes_the_right_args_and_the_prompt_on_stdin_and_returns_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let rec = dir.path().join("rec.txt");
        let cmd = fake_claude(dir.path(), &format!("echo \"$@\" > {r}\ncat >> {r}\necho 完成了", r = rec.display()));
        let out = run_claude(&cmd, "{\"mcpServers\":{}}", "請做事", dir.path(), Duration::from_secs(20)).await;
        assert_eq!(out, Outcome { status: "ok", summary: "完成了\n".into() });
        let rec = std::fs::read_to_string(rec).unwrap();
        assert!(rec.contains("-p") && rec.contains("--allowedTools mcp__aiterm-systems") && rec.contains("--mcp-config"), "{rec}");
        assert!(rec.contains("請做事"), "提示要走標準輸入：{rec}");
        let leftover = std::fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".mcp.json")).count();
        assert_eq!(leftover, 0, "含 token 的 MCP 設定檔用完要刪掉");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_nonzero_exit_is_a_failure_and_a_hang_is_killed_at_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let bad = fake_claude(dir.path(), "echo 出錯了 >&2; exit 3");
        let out = run_claude(&bad, "{}", "x", dir.path(), Duration::from_secs(20)).await;
        assert_eq!(out.status, "failed");
        assert!(out.summary.contains("出錯了") && out.summary.contains('3'), "{}", out.summary);

        let dir2 = tempfile::tempdir().unwrap();
        let hang = fake_claude(dir2.path(), "sleep 30");
        let t0 = std::time::Instant::now();
        let out = run_claude(&hang, "{}", "x", dir2.path(), Duration::from_millis(600)).await;
        assert_eq!(out.status, "timeout");
        assert!(t0.elapsed() < Duration::from_secs(10), "要在逾時就返回，不是等 sleep 結束");

        let out = run_claude("/definitely/not/here", "{}", "x", dir2.path(), Duration::from_secs(5)).await;
        assert_eq!(out.status, "failed");
    }

    /// 整條：排程到點 → 假 claude 以 HTTP 呼叫端點執行配方 → 閘門 → 真 Chrome → 假網站。
    /// 需要真的 Chrome 與網路：`cargo test -p app --lib -- --ignored --test-threads=1 systems::jobs`
    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn a_due_job_runs_claude_which_replays_a_recipe_through_the_gate() {
        use crate::config::ConfigStore;
        use crate::systems::endpoint::EndpointState;
        use crate::systems::recipes::{self, Param};
        use crate::systems::testsite::{conn, fake_site_with_bodies};
        use std::sync::atomic::Ordering;

        let (port, posts, bodies) = fake_site_with_bodies().await;
        let dir = tempfile::tempdir().unwrap();
        let cfg = Arc::new(ConfigStore::new_at(dir.path().join("config.json")));
        let mut c = conn(port, Mode::Autonomous);
        c.id = "t".into();
        cfg.update(|a| a.system_connections.push(c.clone())).unwrap();
        let st = Arc::new(SystemsState::with_pool(cfg.clone(), crate::systems::audit::open_in_memory().await));
        let text = |v: &Value| v["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");

        // 先手動錄一個配方（全部自主，免核准）。
        st.call("t", "sys_navigate", json!({"system":"t","url":format!("http://127.0.0.1:{port}/"),"intent":"讀：開表單"})).await.unwrap();
        let snap = st.call("t", "sys_snapshot", json!({"system":"t"})).await.unwrap();
        let e = recipes::parse_snapshot(&text(&snap));
        let input = e.iter().find(|x| x.role == "textbox").unwrap().reference.clone();
        let button = e.iter().find(|x| x.role == "button").unwrap().reference.clone();
        st.call("t", "sys_type", json!({"system":"t","target":input,"text":"A-102","intent":"寫：填單號"})).await.unwrap();
        st.call("t", "sys_click", json!({"system":"t","target":button,"intent":"寫：送出"})).await.unwrap();
        st.recipe_save("t", "送單", "", vec![Param { name: "order_no".into(), example: "A-102".into() }], None).await.unwrap();
        assert_eq!(posts.load(Ordering::SeqCst), 1);

        // 端點與假 claude（用 curl 呼叫 sys_recipe_run）。
        let ep = Arc::new(EndpointState::default());
        let ep_port = crate::systems::browser::pick_free_port().unwrap();
        ep.start(st.clone(), "tok".into(), ep_port).await.unwrap();
        let out_file = dir.path().join("curl-out.json");
        let script = format!(
            "CFG=\"\"; while [ $# -gt 0 ]; do [ \"$1\" = \"--mcp-config\" ] && CFG=\"$2\"; shift; done\n\
             cat > /dev/null\n\
             URL=$(grep -o '\"url\":\"[^\"]*\"' \"$CFG\" | cut -d'\"' -f4)\n\
             TOK=$(grep -o 'Bearer [^\"]*' \"$CFG\" | cut -d' ' -f2)\n\
             curl -s -H \"Authorization: Bearer $TOK\" -H 'content-type: application/json' -d '{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"sys_recipe_run\",\"arguments\":{{\"system\":\"t\",\"name\":\"送單\",\"params\":{{\"order_no\":\"C-9\"}}}}}}}}' \"$URL\" > {out}\n\
             echo 已執行配方",
            out = out_file.display()
        );
        cfg.update(|a| a.task_board.claude_command = fake_claude(dir.path(), &script)).unwrap();

        let ctx = Arc::new(JobsCtx {
            systems: st.clone(),
            config: cfg.clone(),
            endpoint: { let ep = ep.clone(); Arc::new(move || ep.port().zip(ep.token())) },
            emit: Arc::new(|_, _| {}),
            work_dir: dir.path().join("jobs"),
            timeout: Duration::from_secs(120),
        });
        let pool = st.audit_pool().await.unwrap();
        let now = chrono::Local::now().naive_local();
        let slot = (now - chrono::Duration::minutes(1)).format("%H:%M").to_string();
        let id = create(&pool, &NewJob { system_id: "t".into(), name: "每日送單".into(), goal: "送出今天的單".into(), kind: "daily".into(), spec: Spec { times: vec![slot], ..Default::default() } }).await.unwrap();
        mark_fired(&pool, &id, super::now_ms() - 10 * 60 * 1000).await.unwrap();

        let rt = Arc::new(JobsRuntime::default());
        assert_eq!(rt.tick(&ctx, now).await, 1, "到點的任務要啟動");
        assert_eq!(rt.tick(&ctx, now).await, 0, "同一個時間點不會再啟動一次");
        for _ in 0..240 {
            if !rt.is_running(&id) && list_runs(&pool, &id).await.unwrap().first().is_some_and(|r| r.status != "running") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let run = list_runs(&pool, &id).await.unwrap().remove(0);
        assert_eq!(run.status, "ok", "{}", run.summary);
        assert!(run.summary.contains("已執行配方"));
        assert_eq!(posts.load(Ordering::SeqCst), 2, "排程那一輪的寫入送出了");
        assert!(bodies.lock()[1].contains("a=C-9"), "{:?}", bodies.lock());

        // 改成只讀：再跑一次，寫入被拒絕 → 這一輪算失敗（即使 claude 自己結束碼是 0）。
        let mut ro = c.clone();
        ro.mode = Mode::ReadOnly;
        cfg.update(|a| a.system_connections[0] = ro.clone()).unwrap();
        let out = rt.run_job(&ctx, &id).await.unwrap();
        assert_eq!(out.status, "failed", "{}", out.summary);
        assert!(out.summary.contains("被拒絕"), "{}", out.summary);
        assert_eq!(posts.load(Ordering::SeqCst), 2);

        ep.stop();
        st.close("t").await;
    }

    /// 「請 AI 操作」整條：假 claude 以 HTTP 操作端點（填單號並送出）→ 回傳步驟與參數候選 →
    /// 依候選存成配方 → 用新值重播。需要真的 Chrome：`cargo test -p app --lib -- --ignored --test-threads=1 systems::jobs`
    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn an_adhoc_run_reports_its_steps_and_the_recorded_steps_can_be_saved_and_replayed() {
        use crate::config::ConfigStore;
        use crate::systems::endpoint::EndpointState;
        use crate::systems::recipes::Param;
        use crate::systems::testsite::{conn, fake_site_with_bodies};
        use std::sync::atomic::Ordering;

        let (port, posts, bodies) = fake_site_with_bodies().await;
        let dir = tempfile::tempdir().unwrap();
        let cfg = Arc::new(ConfigStore::new_at(dir.path().join("config.json")));
        let mut c = conn(port, Mode::Autonomous);
        c.id = "t".into();
        cfg.update(|a| a.system_connections.push(c.clone())).unwrap();
        let st = Arc::new(SystemsState::with_pool(cfg.clone(), crate::systems::audit::open_in_memory().await));
        let ep = Arc::new(EndpointState::default());
        let ep_port = crate::systems::browser::pick_free_port().unwrap();
        ep.start(st.clone(), "tok".into(), ep_port).await.unwrap();

        let call = |tool: &str, args: &str| {
            format!(r#"curl -s -H "Authorization: Bearer $TOK" -H 'content-type: application/json' -d '{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}' "$URL""#)
        };
        let snapshot = call("sys_snapshot", r#"{"system":"t"}"#);
        let type_it = call("sys_type", r#"{"system":"t","target":"'$IN'","text":"A-102","intent":"寫：填單號"}"#);
        let click_it = call("sys_click", r#"{"system":"t","target":"'$BT'","intent":"寫：送出"}"#);
        let script = format!(
            "CFG=\"\"; while [ $# -gt 0 ]; do [ \"$1\" = \"--mcp-config\" ] && CFG=\"$2\"; shift; done\n\
             cat > /dev/null\n\
             URL=$(grep -o '\"url\":\"[^\"]*\"' \"$CFG\" | cut -d'\"' -f4)\n\
             TOK=$(grep -o 'Bearer [^\"]*' \"$CFG\" | cut -d' ' -f2)\n\
             IN=\"\"; BT=\"\"; i=0\n\
             while [ -z \"$IN\" ] && [ $i -lt 30 ]; do\n\
               SNAP=$({snapshot})\n\
               IN=$(echo \"$SNAP\" | grep -o 'textbox[^[]*\\[ref=[a-z0-9]*\\]' | head -1 | sed 's/.*ref=\\([a-z0-9]*\\)\\]/\\1/')\n\
               BT=$(echo \"$SNAP\" | grep -o 'button[^[]*\\[ref=[a-z0-9]*\\]' | head -1 | sed 's/.*ref=\\([a-z0-9]*\\)\\]/\\1/')\n\
               i=$((i+1)); [ -z \"$IN\" ] && sleep 0.5\n\
             done\n\
             {type_it} > /dev/null\n\
             {click_it} > /dev/null\n\
             echo 已送出單號 A-102\n\
             echo 結果：成功"
        );
        cfg.update(|a| a.task_board.claude_command = fake_claude(dir.path(), &script)).unwrap();
        let ctx = Arc::new(JobsCtx {
            systems: st.clone(),
            config: cfg.clone(),
            endpoint: { let ep = ep.clone(); Arc::new(move || ep.port().zip(ep.token())) },
            emit: Arc::new(|_, _| {}),
            work_dir: dir.path().join("jobs"),
            timeout: Duration::from_secs(120),
        });
        let rt = Arc::new(JobsRuntime::default());

        assert!(rt.run_adhoc(&ctx, "t", "  ").await.is_err(), "空白的目標不跑");
        assert!(rt.try_lock(&system_key("t")));
        assert_eq!(rt.run_adhoc(&ctx, "t", "填單號 A-102 並送出").await.unwrap_err(), "這個系統正在處理別的事");
        rt.unlock(&system_key("t"));

        let report = rt.run_adhoc(&ctx, "t", "填單號 A-102 並送出").await.unwrap();
        assert_eq!(report.status, "ok", "{}", report.summary);
        assert!(report.summary.contains("已送出單號 A-102") && !report.summary.contains("結果："), "{}", report.summary);
        assert!(report.can_save);
        assert_eq!(report.steps.iter().map(|s| s.tool.as_str()).collect::<Vec<_>>(), vec!["sys_type", "sys_click"]);
        assert!(report.steps.iter().all(|s| s.write));
        assert_eq!(report.params, vec![Param { name: "value1".into(), example: "A-102".into() }], "輸入的值是參數候選");
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        assert!(!rt.is_running(&system_key("t")), "做完要放掉系統的鎖");

        // 依候選存成配方（參數改個好記的名字），再用新值重播。
        let msg = st.recipe_save("t", "送單", "", vec![Param { name: "單號".into(), example: "A-102".into() }], None).await.unwrap();
        assert!(msg.contains("3 步"), "補上回到起始頁的導覽：{msg}");
        assert!(st.journal_steps("t").await.is_empty(), "存完日誌清空");
        let vals: std::collections::HashMap<String, String> = [("單號".to_string(), "B-7".to_string())].into();
        let out = rt.run_recipe_locked(&ctx, "t", "送單", vals).await.unwrap();
        assert!(out.contains("已完成"), "{out}");
        assert_eq!(posts.load(Ordering::SeqCst), 2);
        assert!(bodies.lock()[1].contains("a=B-7"), "{:?}", bodies.lock());

        // 失敗的一輪不留日誌、不能存。
        cfg.update(|a| a.task_board.claude_command = fake_claude(dir.path(), "cat > /dev/null\necho 做不到\necho 結果：失敗：找不到欄位")).unwrap();
        let bad = rt.run_adhoc(&ctx, "t", "做某件事").await.unwrap();
        assert_eq!(bad.status, "failed");
        assert!(!bad.can_save && bad.params.is_empty());
        assert!(st.journal_steps("t").await.is_empty());

        ep.stop();
        st.close("t").await;
    }
}
