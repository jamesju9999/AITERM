//! Agent 協作定時器。見 docs/superpowers/specs/2026-11-16-agent-canvas-scheduler-design.md。
//! 週期規則（`Spec`、`validate`、`due`）在 core，桌面版與 server 共用同一份。
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

pub use aiterm_intake_core::schedule::{due, from_ts, local_ts, validate, Spec, Window, KIND_DAILY, KIND_INTERVAL, KIND_WEEKLY, MAX_TIMES, MIN_INTERVAL_MINUTES};

use chrono::{Local, TimeZone};
use sqlx::SqlitePool;

use crate::tasks::{agents, store, workflow};

pub const MAX_RUNS_KEPT: i64 = 5;
pub const MAX_FAIL_STREAK: i64 = 3;
/// `disabled_reason` 的值；前端 `scheduleStatus.ts` 要跟這些字串一致。
pub const REASON_FAILURES: &str = "連續失敗";
pub const REASON_AGENT_GONE: &str = "角色已不存在";
pub const REASON_AGENT_PAUSED: &str = "角色已暫停";

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ScheduleRow {
    pub id: String,
    pub name: String,
    pub agent_id: String,
    pub instruction: String,
    pub schedule_kind: String,
    pub schedule_json: String,
    pub enabled: bool,
    pub failures_count_as_failed: bool,
    pub canvas_x: Option<f64>,
    pub canvas_y: Option<f64>,
    pub last_fired_at: i64,
    pub last_module_id: Option<String>,
    pub fail_streak: i64,
    pub skip_streak: i64,
    pub disabled_reason: Option<String>,
}

impl ScheduleRow {
    pub fn spec(&self) -> Result<Spec, String> {
        serde_json::from_str(&self.schedule_json).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RunRow {
    pub schedule_id: String,
    pub fired_at: i64,
    pub module_id: Option<String>,
    /// success／failed／skipped
    pub result: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewSchedule {
    pub name: String,
    pub agent_id: String,
    pub instruction: String,
    pub schedule_kind: String,
    pub spec: Spec,
    #[serde(default)]
    pub failures_count_as_failed: bool,
    #[serde(default)]
    pub canvas_x: Option<f64>,
    #[serde(default)]
    pub canvas_y: Option<f64>,
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schedules (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            instruction TEXT NOT NULL,
            schedule_kind TEXT NOT NULL,
            schedule_json TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            failures_count_as_failed INTEGER NOT NULL DEFAULT 0,
            canvas_x REAL,
            canvas_y REAL,
            last_fired_at INTEGER NOT NULL DEFAULT 0,
            last_module_id TEXT,
            fail_streak INTEGER NOT NULL DEFAULT 0,
            skip_streak INTEGER NOT NULL DEFAULT 0,
            disabled_reason TEXT
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schedule_runs (
            schedule_id TEXT NOT NULL,
            fired_at INTEGER NOT NULL,
            module_id TEXT,
            result TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_schedule_runs ON schedule_runs(schedule_id, fired_at)")
        .execute(pool)
        .await?;
    Ok(())
}

fn trimmed(s: &str, what: &str) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        Err(format!("{what}不能是空的"))
    } else {
        Ok(t.to_string())
    }
}

/// `now`：建立當下的 unix 秒，當作 `last_fired_at` 的基準
/// （避免一建立就因為「今天的時間點已經過了」而立刻觸發）。
pub async fn create(pool: &SqlitePool, s: &NewSchedule, now: i64) -> Result<String, String> {
    validate(&s.schedule_kind, &s.spec)?;
    let name = trimmed(&s.name, "名稱")?;
    let instruction = trimmed(&s.instruction, "指令")?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO schedules (id, name, agent_id, instruction, schedule_kind, schedule_json,
                                failures_count_as_failed, canvas_x, canvas_y, last_fired_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(&s.agent_id)
    .bind(instruction)
    .bind(&s.schedule_kind)
    .bind(serde_json::to_string(&s.spec).map_err(|e| e.to_string())?)
    .bind(s.failures_count_as_failed)
    .bind(s.canvas_x)
    .bind(s.canvas_y)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// 更新設定；不動執行狀態（`last_*`、streak）。
pub async fn update(pool: &SqlitePool, id: &str, s: &NewSchedule) -> Result<(), String> {
    validate(&s.schedule_kind, &s.spec)?;
    let name = trimmed(&s.name, "名稱")?;
    let instruction = trimmed(&s.instruction, "指令")?;
    let res = sqlx::query(
        "UPDATE schedules SET name = ?, agent_id = ?, instruction = ?, schedule_kind = ?, schedule_json = ?,
                failures_count_as_failed = ?, canvas_x = ?, canvas_y = ? WHERE id = ?",
    )
    .bind(name)
    .bind(&s.agent_id)
    .bind(instruction)
    .bind(&s.schedule_kind)
    .bind(serde_json::to_string(&s.spec).map_err(|e| e.to_string())?)
    .bind(s.failures_count_as_failed)
    .bind(s.canvas_x)
    .bind(s.canvas_y)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    if res.rows_affected() == 0 {
        return Err("找不到這個定時器".into());
    }
    Ok(())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<ScheduleRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM schedules ORDER BY rowid").fetch_all(pool).await
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<ScheduleRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM schedules WHERE id = ?").bind(id).fetch_optional(pool).await
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM schedule_runs WHERE schedule_id = ?").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM schedules WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

/// 啟用時清掉連續失敗／跳過與停用原因，並把基準時間設成現在
/// （停用期間錯過的時間點不補，也避免一啟用就立刻觸發）。
pub async fn set_enabled(pool: &SqlitePool, id: &str, enabled: bool, now: i64) -> Result<(), sqlx::Error> {
    if enabled {
        sqlx::query(
            "UPDATE schedules SET enabled = 1, fail_streak = 0, skip_streak = 0, disabled_reason = NULL, last_fired_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("UPDATE schedules SET enabled = 0, disabled_reason = NULL WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn record_run(pool: &SqlitePool, id: &str, fired_at: i64, module_id: Option<&str>, result: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO schedule_runs (schedule_id, fired_at, module_id, result) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind(fired_at)
        .bind(module_id)
        .bind(result)
        .execute(pool)
        .await?;
    sqlx::query(
        "DELETE FROM schedule_runs WHERE schedule_id = ?1 AND rowid NOT IN
           (SELECT rowid FROM schedule_runs WHERE schedule_id = ?1 ORDER BY fired_at DESC, rowid DESC LIMIT ?2)",
    )
    .bind(id)
    .bind(MAX_RUNS_KEPT)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_runs(pool: &SqlitePool, id: &str) -> Result<Vec<RunRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM schedule_runs WHERE schedule_id = ? ORDER BY fired_at DESC, rowid DESC")
        .bind(id)
        .fetch_all(pool)
        .await
}

// ── 建一輪 ────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum Fired {
    Created { card_id: String },
    Skipped,
}

pub fn card_title(name: &str) -> String {
    format!("定時：{name}")
}

fn work_body(instruction: &str) -> String {
    format!(
        "這是定時觸發的工作（每次觸發都是新的一輪，請依下面的指示完成，並照一般流程交付）：\n\n{}\n",
        instruction.trim()
    )
}

/// 建一輪。`head_sha`：目標是驗收者時必填（受驗版本），其他角色忽略。
/// 上一輪還沒結案 → 跳過（記一筆 skipped、`skip_streak`+1），並照樣推進 `last_fired_at`，
/// 不然每分鐘都會再試一次。
pub async fn fire(pool: &SqlitePool, project_path: &std::path::Path, row: &ScheduleRow, now: i64, head_sha: Option<&str>) -> Result<Fired, String> {
    let agent = agents::get_agent(pool, &row.agent_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "目標角色已不存在".to_string())?;
    if round_open(pool, row).await? {
        sqlx::query("UPDATE schedules SET skip_streak = skip_streak + 1, last_fired_at = ? WHERE id = ?")
            .bind(now)
            .bind(&row.id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
        record_run(pool, &row.id, now, row.last_module_id.as_deref(), "skipped").await.map_err(|e| e.to_string())?;
        return Ok(Fired::Skipped);
    }
    let title = card_title(&row.name);
    let card_id = match agent.kind.as_str() {
        "coder" => workflow::create_scheduled_work_card(pool, &agent, &title, &work_body(&row.instruction)).await,
        "verifier" => {
            let sha = head_sha.ok_or_else(|| "驗收卡需要受驗版本（HEAD）".to_string())?;
            workflow::create_scheduled_verify_card(pool, project_path, &agent, &title, &row.instruction, sha).await
        }
        other => return Err(format!("定時器只能叫起工程師或驗收者，不能是：{other}")),
    }
    .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE schedules SET last_fired_at = ?, last_module_id = ?, skip_streak = 0 WHERE id = ?")
        .bind(now)
        .bind(&card_id)
        .bind(&row.id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Fired::Created { card_id })
}

// ── 結算一輪 ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundResult {
    Pending,
    Success,
    Failed,
    /// 沒有進行中的輪（或那張卡被刪了）。
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SettleEvent {
    Succeeded,
    Failed,
    AutoDisabled,
}

/// 上一輪現在的狀態。工程師起點看模組鏈（審查通過＝成功；合併與驗收是專案層級，不算在單一輪裡）；
/// 驗收者起點看那張驗收卡自己的結果。
pub async fn round_result(pool: &SqlitePool, row: &ScheduleRow) -> Result<RoundResult, String> {
    let Some(id) = row.last_module_id.as_deref() else { return Ok(RoundResult::Gone) };
    let Some(card) = store::get_task(pool, id).await.map_err(|e| e.to_string())? else { return Ok(RoundResult::Gone) };
    if card.card_kind == workflow::KIND_VERIFY {
        if card.status != store::STATUS_DONE {
            return Ok(RoundResult::Pending);
        }
        if card.outcome.as_deref() != Some("success") {
            return Ok(RoundResult::Failed);
        }
        return Ok(match card.verdict.as_deref() {
            Some("fail") | Some("attention") if row.failures_count_as_failed => RoundResult::Failed,
            _ => RoundResult::Success,
        });
    }
    let tasks = store::list_tasks(pool).await.map_err(|e| e.to_string())?;
    let chain: Vec<store::TaskRow> = tasks.iter().filter(|t| t.module_id.as_deref() == Some(card.id.as_str())).cloned().collect();
    let has_reviewer = agents::list_agents(pool).await.map_err(|e| e.to_string())?.iter().any(|a| a.kind == "reviewer");
    use workflow::ModuleState as M;
    Ok(match workflow::module_state(&card, &chain, has_reviewer) {
        // 工作卡剛完成、審查卡還沒建出來的空檔，`module_state` 會先判成通過（它只看鏈尾）。
        // 有審核者（且不是快速路徑、也沒被人工放行）卻還沒有任何審查卡時，要等審查卡出現再判。
        M::Passed
            if has_reviewer
                && !card.fast
                && !card.module_override
                && !chain.iter().any(|c| c.card_kind == workflow::KIND_REVIEW) =>
        {
            RoundResult::Pending
        }
        M::Passed => RoundResult::Success,
        M::Attention | M::Failed | M::ReviewFailed | M::DispatchFailed => RoundResult::Failed,
        _ => RoundResult::Pending,
    })
}

pub async fn round_open(pool: &SqlitePool, row: &ScheduleRow) -> Result<bool, String> {
    Ok(round_result(pool, row).await? == RoundResult::Pending)
}

/// 停用並記下原因（`set_enabled(.., true, ..)` 會清掉原因與連續失敗／跳過）。
async fn disable(pool: &SqlitePool, id: &str, reason: &str) -> Result<(), String> {
    sqlx::query("UPDATE schedules SET enabled = 0, disabled_reason = ? WHERE id = ?")
        .bind(reason)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 判定一輪並更新狀態。回傳這次發生的事（空＝還在進行，或沒有進行中的輪）。
pub async fn settle(pool: &SqlitePool, id: &str, now: i64) -> Result<Vec<SettleEvent>, String> {
    let Some(row) = get(pool, id).await.map_err(|e| e.to_string())? else { return Ok(vec![]) };
    let module_id = row.last_module_id.clone();
    let mut events = Vec::new();
    match round_result(pool, &row).await? {
        RoundResult::Pending => {}
        RoundResult::Gone => {
            if module_id.is_some() {
                sqlx::query("UPDATE schedules SET last_module_id = NULL WHERE id = ?").bind(id).execute(pool).await.map_err(|e| e.to_string())?;
            }
        }
        RoundResult::Success => {
            sqlx::query("UPDATE schedules SET fail_streak = 0, last_module_id = NULL WHERE id = ?").bind(id).execute(pool).await.map_err(|e| e.to_string())?;
            record_run(pool, id, now, module_id.as_deref(), "success").await.map_err(|e| e.to_string())?;
            if let Some(m) = module_id.as_deref() {
                // 模組本身加整條鏈；只歸檔已完成的（歸檔只是看板上的可見性，不碰進行中的卡）。
                sqlx::query("UPDATE tasks SET archived_at = ?1 WHERE (id = ?2 OR module_id = ?2) AND status = 'done' AND archived_at IS NULL")
                    .bind(now)
                    .bind(m)
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            events.push(SettleEvent::Succeeded);
        }
        RoundResult::Failed => {
            let streak = row.fail_streak + 1;
            sqlx::query("UPDATE schedules SET fail_streak = ?, last_module_id = NULL WHERE id = ?").bind(streak).bind(id).execute(pool).await.map_err(|e| e.to_string())?;
            record_run(pool, id, now, module_id.as_deref(), "failed").await.map_err(|e| e.to_string())?;
            events.push(SettleEvent::Failed);
            if streak >= MAX_FAIL_STREAK {
                disable(pool, id, REASON_FAILURES).await?;
                events.push(SettleEvent::AutoDisabled);
            }
        }
    }
    Ok(events)
}

// ── 掃描 ──────────────────────────────────────────────────────────────────

pub struct TickCtx {
    pub project_path: std::path::PathBuf,
    /// 驗收者起點需要的受驗版本；沒有就叫不起驗收者（那一輪會記錄錯誤、下一分鐘再試）。
    pub head_sha: Option<String>,
}

#[derive(Debug, Default)]
pub struct TickReport {
    pub fired: Vec<String>,
    pub events: Vec<(String, SettleEvent)>,
}

/// 掃過所有定時器一次：先結算上一輪，再依 `due` 決定要不要建新的一輪。
pub async fn tick(pool: &SqlitePool, ctx: &TickCtx, now: NaiveDateTime) -> Result<TickReport, String> {
    let mut report = TickReport::default();
    let now_ts = local_ts(now);
    for row in list(pool).await.map_err(|e| e.to_string())? {
        for e in settle(pool, &row.id, now_ts).await? {
            report.events.push((row.id.clone(), e));
        }
        let Some(row) = get(pool, &row.id).await.map_err(|e| e.to_string())? else { continue };
        if !row.enabled {
            continue;
        }
        let Some(agent) = agents::get_agent(pool, &row.agent_id).await.map_err(|e| e.to_string())? else {
            disable(pool, &row.id, REASON_AGENT_GONE).await?;
            continue;
        };
        let Ok(spec) = row.spec() else { continue };
        if !due(&row.schedule_kind, &spec, from_ts(row.last_fired_at), now) {
            continue;
        }
        // 到點才檢查：Agent 因為卡片失敗被暫停（要人決定重試或跳過）時，它不會再做事，
        // 建了卡也只會一直排著、下一輪被跳過。直接停用並通知，等人處理好再重新啟用。
        // 這跟使用者按的「暫停」（manual_pause）是兩回事，那個不動定時器。
        if agent.paused_task_id.is_some() {
            disable(pool, &row.id, REASON_AGENT_PAUSED).await?;
            report.events.push((row.id.clone(), SettleEvent::AutoDisabled));
            continue;
        }
        match fire(pool, &ctx.project_path, &row, now_ts, ctx.head_sha.as_deref()).await {
            Ok(Fired::Created { .. }) => report.fired.push(row.id.clone()),
            Ok(Fired::Skipped) => {}
            Err(e) => eprintln!("schedule {} fire: {e}", row.name),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }
    fn daily(times: &[&str]) -> Spec {
        Spec { times: times.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    // ── 資料存取 ──────────────────────────────────────────────────────────

    async fn pool() -> SqlitePool {
        let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&p).await.unwrap();
        p
    }

    async fn make_agent(pool: &SqlitePool, kind: &str) -> String {
        agents::create_agent(
            pool,
            &agents::NewAgent { kind, name: kind, role: kind, prompt_prefix: "", project_dir: "/tmp/repo", canvas_x: 0.0, canvas_y: 0.0 },
        )
        .await
        .unwrap()
    }

    fn new_schedule(agent: &str) -> NewSchedule {
        NewSchedule {
            name: "每日測試".into(),
            agent_id: agent.into(),
            instruction: "跑完整測試".into(),
            schedule_kind: KIND_DAILY.into(),
            spec: daily(&["08:00"]),
            failures_count_as_failed: false,
            canvas_x: None,
            canvas_y: None,
        }
    }

    const P: &str = "/tmp/p";

    #[tokio::test]
    async fn create_then_list_roundtrips_the_spec() {
        let pool = pool().await;
        let id = create(&pool, &new_schedule("a1"), 1_000).await.unwrap();
        let rows = list(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].spec().unwrap(), daily(&["08:00"]));
        assert_eq!(rows[0].last_fired_at, 1_000, "建立時以現在當基準");
        assert!(rows[0].enabled);
    }

    #[tokio::test]
    async fn create_rejects_an_invalid_spec_and_blank_fields() {
        let pool = pool().await;
        let mut s = new_schedule("a1");
        s.spec = Spec::default();
        assert!(create(&pool, &s, 0).await.is_err());
        let mut s = new_schedule("a1");
        s.name = "  ".into();
        assert!(create(&pool, &s, 0).await.is_err());
        let mut s = new_schedule("a1");
        s.instruction = "".into();
        assert!(create(&pool, &s, 0).await.is_err());
    }

    #[tokio::test]
    async fn update_changes_settings_but_not_run_state() {
        let pool = pool().await;
        let id = create(&pool, &new_schedule("a1"), 1_000).await.unwrap();
        sqlx::query("UPDATE schedules SET fail_streak = 2, last_module_id = 'm' WHERE id = ?").bind(&id).execute(&pool).await.unwrap();
        let mut s = new_schedule("a2");
        s.name = "改名".into();
        update(&pool, &id, &s).await.unwrap();
        let r = get(&pool, &id).await.unwrap().unwrap();
        assert_eq!((r.name.as_str(), r.agent_id.as_str()), ("改名", "a2"));
        assert_eq!(r.fail_streak, 2);
        assert_eq!(r.last_module_id.as_deref(), Some("m"));
        assert!(update(&pool, "nope", &s).await.is_err());
    }

    #[tokio::test]
    async fn enabling_resets_streaks_reason_and_baseline() {
        let pool = pool().await;
        let id = create(&pool, &new_schedule("a1"), 1_000).await.unwrap();
        sqlx::query("UPDATE schedules SET enabled = 0, fail_streak = 3, skip_streak = 2, disabled_reason = '連續失敗' WHERE id = ?")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        set_enabled(&pool, &id, true, 5_000).await.unwrap();
        let r = get(&pool, &id).await.unwrap().unwrap();
        assert!(r.enabled && r.fail_streak == 0 && r.skip_streak == 0 && r.disabled_reason.is_none());
        assert_eq!(r.last_fired_at, 5_000);
    }

    #[tokio::test]
    async fn runs_keep_only_the_latest_five_and_delete_removes_them() {
        let pool = pool().await;
        let id = create(&pool, &new_schedule("a1"), 0).await.unwrap();
        for i in 0..7 {
            record_run(&pool, &id, i, None, "success").await.unwrap();
        }
        let runs = list_runs(&pool, &id).await.unwrap();
        assert_eq!(runs.len(), 5);
        assert_eq!(runs[0].fired_at, 6, "新的在前");
        delete(&pool, &id).await.unwrap();
        assert!(list(&pool).await.unwrap().is_empty());
        assert!(list_runs(&pool, &id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn old_databases_gain_the_schedule_tables() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        sqlx::query("DROP TABLE schedules").execute(&pool).await.unwrap();
        sqlx::query("DROP TABLE schedule_runs").execute(&pool).await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        assert!(list(&pool).await.unwrap().is_empty());
    }

    // ── fire ──────────────────────────────────────────────────────────────

    async fn fire_new(pool: &SqlitePool, sid: &str, now: i64, head: Option<&str>) -> Result<Fired, String> {
        let row = get(pool, sid).await.unwrap().unwrap();
        fire(pool, std::path::Path::new(P), &row, now, head).await
    }

    #[tokio::test]
    async fn firing_for_a_coder_creates_a_queued_work_card_owned_by_it() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        let Fired::Created { card_id } = fire_new(&pool, &sid, 10_000, None).await.unwrap() else { panic!("應建卡") };
        let card = store::get_task(&pool, &card_id).await.unwrap().unwrap();
        assert_eq!(card.card_kind, "work");
        assert_eq!(card.status, "queued");
        assert_eq!(card.agent_id.as_deref(), Some(coder.as_str()));
        assert!(card.module_id.is_none(), "模組本身 module_id 為空");
        assert!(card.body.contains("跑完整測試"), "內文帶固定指令");
        let after = get(&pool, &sid).await.unwrap().unwrap();
        assert_eq!(after.last_fired_at, 10_000);
        assert_eq!(after.last_module_id.as_deref(), Some(card_id.as_str()));
        assert!(list_runs(&pool, &sid).await.unwrap().is_empty(), "成敗在 settle 時才記");
    }

    #[tokio::test]
    async fn firing_for_a_verifier_creates_a_verify_card_pinned_to_head() {
        let pool = pool().await;
        let v = make_agent(&pool, "verifier").await;
        let sid = create(&pool, &new_schedule(&v), 0).await.unwrap();
        assert!(fire_new(&pool, &sid, 10_000, None).await.is_err(), "沒有 HEAD 就建不了驗收卡");
        assert!(store::list_tasks(&pool).await.unwrap().is_empty());
        let Fired::Created { card_id } = fire_new(&pool, &sid, 10_000, Some("abc123")).await.unwrap() else { panic!() };
        let card = store::get_task(&pool, &card_id).await.unwrap().unwrap();
        assert_eq!(card.card_kind, "verify");
        assert_eq!(card.end_sha.as_deref(), Some("abc123"));
        assert!(card.review_of.is_none());
        assert!(card.body.contains("跑完整測試"));
    }

    #[tokio::test]
    async fn firing_is_skipped_while_the_previous_round_is_open() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        assert!(matches!(fire_new(&pool, &sid, 10_000, None).await.unwrap(), Fired::Created { .. }));
        assert_eq!(fire_new(&pool, &sid, 20_000, None).await.unwrap(), Fired::Skipped);
        let after = get(&pool, &sid).await.unwrap().unwrap();
        assert_eq!(after.skip_streak, 1);
        assert_eq!(after.last_fired_at, 20_000, "跳過也推進基準");
        assert_eq!(list_runs(&pool, &sid).await.unwrap()[0].result, "skipped");
        assert_eq!(store::list_tasks(&pool).await.unwrap().len(), 1, "沒有多建卡");
    }

    #[tokio::test]
    async fn firing_for_a_missing_or_wrong_kind_agent_creates_nothing() {
        let pool = pool().await;
        let sid = create(&pool, &new_schedule("ghost"), 0).await.unwrap();
        assert!(fire_new(&pool, &sid, 10_000, None).await.is_err());
        let reviewer = make_agent(&pool, "reviewer").await;
        let sid2 = create(&pool, &new_schedule(&reviewer), 0).await.unwrap();
        assert!(fire_new(&pool, &sid2, 10_000, None).await.is_err());
        assert!(store::list_tasks(&pool).await.unwrap().is_empty());
        assert_eq!(get(&pool, &sid).await.unwrap().unwrap().last_fired_at, 0, "失敗不推進基準");
    }

    // ── settle ────────────────────────────────────────────────────────────

    async fn finish_card(pool: &SqlitePool, id: &str, outcome: &str, verdict: Option<&str>) {
        sqlx::query("UPDATE tasks SET status = 'done', outcome = ?, verdict = ? WHERE id = ?")
            .bind(outcome)
            .bind(verdict)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn round(pool: &SqlitePool, sid: &str, now: i64, head: Option<&str>) -> String {
        let Fired::Created { card_id } = fire_new(pool, sid, now, head).await.unwrap() else { panic!("應建卡") };
        card_id
    }

    #[tokio::test]
    async fn a_passed_coder_round_succeeds_and_archives_its_chain() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        let card = round(&pool, &sid, 10, None).await;
        // 沒有審核者：work 卡成功結案＝模組通過。
        finish_card(&pool, &card, "success", None).await;
        assert_eq!(settle(&pool, &sid, 20).await.unwrap(), vec![SettleEvent::Succeeded]);
        let after = get(&pool, &sid).await.unwrap().unwrap();
        assert_eq!(after.fail_streak, 0);
        assert!(after.last_module_id.is_none(), "已結案，下一輪可以建");
        assert!(store::get_task(&pool, &card).await.unwrap().unwrap().archived_at.is_some());
        assert_eq!(list_runs(&pool, &sid).await.unwrap()[0].result, "success");
    }

    #[tokio::test]
    async fn archiving_leaves_unfinished_chain_cards_alone() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        let card = round(&pool, &sid, 10, None).await;
        finish_card(&pool, &card, "success", None).await;
        sqlx::query(
            "INSERT INTO tasks (id, title, body, project_dir, status, module_id, card_kind) VALUES ('extra', 't', '', '/tmp/repo', 'queued', ?, 'fix')",
        )
        .bind(&card)
        .execute(&pool)
        .await
        .unwrap();
        settle(&pool, &sid, 20).await.unwrap();
        assert!(store::get_task(&pool, "extra").await.unwrap().unwrap().archived_at.is_none());
    }

    #[tokio::test]
    async fn a_failed_round_counts_and_keeps_the_cards_visible() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        let card = round(&pool, &sid, 10, None).await;
        finish_card(&pool, &card, "failed", None).await;
        assert_eq!(settle(&pool, &sid, 20).await.unwrap(), vec![SettleEvent::Failed]);
        assert_eq!(get(&pool, &sid).await.unwrap().unwrap().fail_streak, 1);
        assert!(store::get_task(&pool, &card).await.unwrap().unwrap().archived_at.is_none());
    }

    #[tokio::test]
    async fn three_failures_in_a_row_disable_the_schedule() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        let mut last = Vec::new();
        for i in 0..3 {
            let card = round(&pool, &sid, 10 + i, None).await;
            finish_card(&pool, &card, "failed", None).await;
            last = settle(&pool, &sid, 20 + i).await.unwrap();
        }
        assert_eq!(last, vec![SettleEvent::Failed, SettleEvent::AutoDisabled]);
        let after = get(&pool, &sid).await.unwrap().unwrap();
        assert!(!after.enabled);
        assert_eq!(after.disabled_reason.as_deref(), Some("連續失敗"));
    }

    #[tokio::test]
    async fn two_failures_do_not_disable_and_a_success_resets_the_streak() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        for i in 0..2 {
            let card = round(&pool, &sid, 10 + i, None).await;
            finish_card(&pool, &card, "failed", None).await;
            settle(&pool, &sid, 20 + i).await.unwrap();
        }
        assert!(get(&pool, &sid).await.unwrap().unwrap().enabled);
        let card = round(&pool, &sid, 30, None).await;
        finish_card(&pool, &card, "success", None).await;
        settle(&pool, &sid, 40).await.unwrap();
        assert_eq!(get(&pool, &sid).await.unwrap().unwrap().fail_streak, 0);
    }

    #[tokio::test]
    async fn a_pending_round_settles_to_nothing() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        round(&pool, &sid, 10, None).await;
        assert!(settle(&pool, &sid, 20).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn verify_round_respects_the_failures_switch() {
        for (count_as_failed, expect) in [(false, SettleEvent::Succeeded), (true, SettleEvent::Failed)] {
            let pool = pool().await;
            let v = make_agent(&pool, "verifier").await;
            let mut ns = new_schedule(&v);
            ns.failures_count_as_failed = count_as_failed;
            let sid = create(&pool, &ns, 0).await.unwrap();
            let card = round(&pool, &sid, 10, Some("abc")).await;
            finish_card(&pool, &card, "success", Some("fail")).await;
            assert_eq!(settle(&pool, &sid, 20).await.unwrap()[0], expect, "switch={count_as_failed}");
        }
    }

    #[tokio::test]
    async fn a_verify_card_that_errored_is_a_failed_round() {
        let pool = pool().await;
        let v = make_agent(&pool, "verifier").await;
        let sid = create(&pool, &new_schedule(&v), 0).await.unwrap();
        let card = round(&pool, &sid, 10, Some("abc")).await;
        finish_card(&pool, &card, "failed", None).await;
        assert_eq!(settle(&pool, &sid, 20).await.unwrap()[0], SettleEvent::Failed);
    }

    #[tokio::test]
    async fn a_deleted_round_card_just_closes_the_round() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        let card = round(&pool, &sid, 10, None).await;
        store::delete_task(&pool, &card).await.unwrap();
        assert!(settle(&pool, &sid, 20).await.unwrap().is_empty());
        let after = get(&pool, &sid).await.unwrap().unwrap();
        assert!(after.last_module_id.is_none());
        assert_eq!(after.fail_streak, 0);
    }

    #[tokio::test]
    async fn with_a_reviewer_the_round_waits_for_the_review_and_succeeds_only_when_it_passes() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        make_agent(&pool, "reviewer").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        let module = round(&pool, &sid, 10, None).await;
        finish_card(&pool, &module, "success", None).await;
        // 工作做完、審查還沒開始：還在進行中，不能當成功（也不能歸檔）。
        assert!(settle(&pool, &sid, 20).await.unwrap().is_empty());
        assert!(store::get_task(&pool, &module).await.unwrap().unwrap().archived_at.is_none());
        // 審查通過 → 成功，模組與審查卡一起歸檔。
        sqlx::query(
            "INSERT INTO tasks (id, title, body, project_dir, status, outcome, verdict, module_id, review_of, card_kind)
             VALUES ('rv1', 'r', '', '/tmp/repo', 'done', 'success', 'pass', ?1, ?1, 'review')",
        )
        .bind(&module)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(settle(&pool, &sid, 30).await.unwrap(), vec![SettleEvent::Succeeded]);
        assert!(store::get_task(&pool, &module).await.unwrap().unwrap().archived_at.is_some());
        assert!(store::get_task(&pool, "rv1").await.unwrap().unwrap().archived_at.is_some());
    }

    #[tokio::test]
    async fn with_a_reviewer_a_review_that_failed_without_a_verdict_is_a_failed_round() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        make_agent(&pool, "reviewer").await;
        let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
        let module = round(&pool, &sid, 10, None).await;
        finish_card(&pool, &module, "success", None).await;
        sqlx::query(
            "INSERT INTO tasks (id, title, body, project_dir, status, outcome, module_id, review_of, card_kind)
             VALUES ('rv1', 'r', '', '/tmp/repo', 'done', 'failed', ?1, ?1, 'review')",
        )
        .bind(&module)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(settle(&pool, &sid, 30).await.unwrap(), vec![SettleEvent::Failed]);
    }

    // ── tick ──────────────────────────────────────────────────────────────

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn ctx() -> TickCtx {
        TickCtx { project_path: std::path::PathBuf::from(P), head_sha: None }
    }

    #[tokio::test]
    async fn tick_fires_a_due_schedule_exactly_once() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
        assert!(tick(&pool, &ctx(), at("2026-11-16 07:59")).await.unwrap().fired.is_empty());
        assert_eq!(tick(&pool, &ctx(), at("2026-11-16 08:00")).await.unwrap().fired, vec![sid]);
        assert!(tick(&pool, &ctx(), at("2026-11-16 08:01")).await.unwrap().fired.is_empty(), "同一個時間點不重複");
        assert_eq!(store::list_tasks(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn tick_ignores_disabled_schedules() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
        set_enabled(&pool, &sid, false, 0).await.unwrap();
        assert!(tick(&pool, &ctx(), at("2026-11-16 09:00")).await.unwrap().fired.is_empty());
    }

    #[tokio::test]
    async fn tick_disables_schedules_whose_agent_is_gone() {
        let pool = pool().await;
        let sid = create(&pool, &new_schedule("ghost"), local_ts(at("2026-11-15 20:00"))).await.unwrap();
        tick(&pool, &ctx(), at("2026-11-16 09:00")).await.unwrap();
        let r = get(&pool, &sid).await.unwrap().unwrap();
        assert!(!r.enabled);
        assert_eq!(r.disabled_reason.as_deref(), Some("角色已不存在"));
    }

    #[tokio::test]
    async fn tick_settles_a_finished_round_then_fires_the_next_in_the_same_pass() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
        tick(&pool, &ctx(), at("2026-11-16 08:00")).await.unwrap();
        let card = get(&pool, &sid).await.unwrap().unwrap().last_module_id.unwrap();
        finish_card(&pool, &card, "success", None).await;
        let r = tick(&pool, &ctx(), at("2026-11-17 08:00")).await.unwrap();
        assert!(r.events.iter().any(|(_, e)| *e == SettleEvent::Succeeded));
        assert_eq!(r.fired, vec![sid], "前一輪已結案，今天這輪能建、不會被跳過");
    }

    #[tokio::test]
    async fn tick_skips_while_the_previous_round_is_still_open() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
        tick(&pool, &ctx(), at("2026-11-16 08:00")).await.unwrap();
        let r = tick(&pool, &ctx(), at("2026-11-17 08:00")).await.unwrap();
        assert!(r.fired.is_empty());
        assert_eq!(get(&pool, &sid).await.unwrap().unwrap().skip_streak, 1);
    }

    #[tokio::test]
    async fn tick_disables_a_due_schedule_whose_agent_is_paused_for_a_failed_card() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
        agents::set_paused(&pool, &coder, Some("some-failed-card")).await.unwrap();
        let r = tick(&pool, &ctx(), at("2026-11-16 08:00")).await.unwrap();
        assert!(r.fired.is_empty(), "被暫停的工程師不會做事，不建卡");
        assert!(store::list_tasks(&pool).await.unwrap().is_empty());
        let row = get(&pool, &sid).await.unwrap().unwrap();
        assert!(!row.enabled);
        assert_eq!(row.disabled_reason.as_deref(), Some(REASON_AGENT_PAUSED));
        assert!(r.events.iter().any(|(id, e)| id == &sid && *e == SettleEvent::AutoDisabled), "要通知使用者");
    }

    #[tokio::test]
    async fn a_paused_agent_does_not_disable_the_schedule_before_it_is_due() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
        agents::set_paused(&pool, &coder, Some("some-failed-card")).await.unwrap();
        tick(&pool, &ctx(), at("2026-11-16 07:59")).await.unwrap();
        assert!(get(&pool, &sid).await.unwrap().unwrap().enabled);
    }

    #[tokio::test]
    async fn a_failed_round_that_paused_the_agent_is_counted_then_the_next_due_time_disables() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
        tick(&pool, &ctx(), at("2026-11-16 08:00")).await.unwrap();
        let card = get(&pool, &sid).await.unwrap().unwrap().last_module_id.unwrap();
        finish_card(&pool, &card, "failed", None).await;
        agents::set_paused(&pool, &coder, Some(&card)).await.unwrap(); // 真實流程：失敗的卡會暫停 Agent
        let r = tick(&pool, &ctx(), at("2026-11-17 08:00")).await.unwrap();
        assert!(r.events.iter().any(|(_, e)| *e == SettleEvent::Failed));
        let row = get(&pool, &sid).await.unwrap().unwrap();
        assert_eq!(row.fail_streak, 1, "失敗那一輪照樣記帳");
        assert!(!row.enabled);
        assert_eq!(row.disabled_reason.as_deref(), Some(REASON_AGENT_PAUSED));
        assert_eq!(store::list_tasks(&pool).await.unwrap().len(), 1, "沒有為被暫停的工程師再建卡");
    }

    #[tokio::test]
    async fn re_enabling_after_unpausing_works_again() {
        let pool = pool().await;
        let coder = make_agent(&pool, "coder").await;
        let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
        agents::set_paused(&pool, &coder, Some("x")).await.unwrap();
        tick(&pool, &ctx(), at("2026-11-16 08:00")).await.unwrap();
        agents::set_paused(&pool, &coder, None).await.unwrap();
        set_enabled(&pool, &sid, true, local_ts(at("2026-11-16 09:00"))).await.unwrap();
        assert!(tick(&pool, &ctx(), at("2026-11-17 08:00")).await.unwrap().fired.contains(&sid));
    }
}
