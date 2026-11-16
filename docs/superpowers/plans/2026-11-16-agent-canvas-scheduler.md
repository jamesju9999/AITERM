# Agent 協作定時器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Agent 協作桌布上加「定時器」：依每天／每週／固定間隔建一輪工作交給指定角色，之後沿用現有交接。

**Architecture:** 每專案 `tasks.db` 新增 `schedules`／`schedule_runs`。`tasks/schedules.rs` 放純函式 `due`、資料存取、`fire`（建模組＋work 卡，或建 verify 卡）、`settle`（判定上一輪成敗、累計失敗、成功自動歸檔）。`scheduler::spawn` 另起每分鐘一次的迴圈呼叫 `schedules::tick`。前端新增 `ScheduleNode`／`ScheduleDialog`，不碰 `EdgeLayer`。

**Tech Stack:** Rust（sqlx／sqlite、chrono、tokio）、Tauri 2 IPC、React 19、Vitest。

**對照 spec 的查證結果（spec §8 的三個待確認項）：**
1. 「模組」＝一張 `card_kind='work'`、`module_id` 為空的卡，用 `store::create_task`＋`agents::assign_task_with` 建立。審查鏈靠 `module_id` 串。
2. 驗收卡原本掛在合併卡之後（`review_of = 合併卡`、`end_sha = 合併結果`）。定時器直接建 verify 卡時沒有 `end_sha`，`prepare_review_worktree` 會回錯，所以 **fire 要把 `end_sha` 設成該角色 repo 的目前 `HEAD`**（`GitClient::head_sha`）。
3. 工程師起點的「成功」＝ `module_state == Passed`（審查通過）。合併與驗收是專案層級、要等所有模組都過，不屬於單一輪，所以**不等它**；spec §5「驗收也要過」對工程師起點改為「審查通過」（Task 11 同步修 spec）。驗收者起點仍看 verify 卡自己的結果。驗收失敗時既有的 `on_verify_failed`（開修正卡給模組工程師／協調者）照常運作，這是「套用後面流程」的一部分。

---

## 檔案結構

- Create `src-tauri/src/tasks/schedules.rs`：型別、`validate`、`due`、表與 CRUD、`fire`、`settle`、`tick`。
- Modify `src-tauri/src/tasks/mod.rs`：`pub mod schedules;`、`init_schema` 末尾呼叫 `schedules::init_schema`。
- Modify `src-tauri/src/tasks/scheduler.rs`：`spawn` 內起每分鐘的 `schedules::tick` 迴圈。
- Create `src-tauri/src/commands/schedules.rs`；Modify `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs`（註冊指令）。
- Create `src/ipc/schedules.ts`（＋`.test.ts`）。
- Create `src/components/AgentCanvas/scheduleSummary.ts`（＋test）、`ScheduleDialog.tsx`（＋test）、`ScheduleNode.tsx`（＋test）。
- Modify `src/components/AgentCanvas/TopMoreMenu.tsx`、`index.tsx`、`index.css`、`src/lib/i18n.ts`。
- Modify `docs/superpowers/specs/2026-11-16-agent-canvas-scheduler-design.md`（Task 11）。

測試指令：Rust `cd src-tauri && cargo test --workspace --no-fail-fast schedules`；前端 `npx vitest run <path>`；型別 `npx tsc -b`。

---

### Task 1: 型別、驗證、`due` 純函式

**Files:**
- Create: `src-tauri/src/tasks/schedules.rs`
- Modify: `src-tauri/src/tasks/mod.rs`（加 `pub mod schedules;`）

- [ ] **Step 1: 寫失敗測試與骨架**

建立 `schedules.rs`，先放型別、`validate` 與 `due` 的簽章（函式本體 `todo!()`），以及下列測試：

```rust
//! Agent 協作定時器。見 docs/superpowers/specs/2026-11-16-agent-canvas-scheduler-design.md。
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

pub const KIND_DAILY: &str = "daily";
pub const KIND_WEEKLY: &str = "weekly";
pub const KIND_INTERVAL: &str = "interval";
pub const MAX_TIMES: usize = 12;
pub const MIN_INTERVAL_MINUTES: u32 = 5;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub from: String,
    pub to: String,
}

/// `schedule_json` 的內容；三種模式共用一個結構，各自只用自己的欄位。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    #[serde(default)]
    pub times: Vec<String>,
    /// 1=週一 … 7=週日。
    #[serde(default)]
    pub days: Vec<u32>,
    #[serde(default)]
    pub every_minutes: u32,
    #[serde(default)]
    pub window: Option<Window>,
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

pub fn validate(kind: &str, spec: &Spec) -> Result<(), String> {
    let check_times = |spec: &Spec| -> Result<(), String> {
        if spec.times.is_empty() {
            return Err("至少要設定一個時間點".into());
        }
        if spec.times.len() > MAX_TIMES {
            return Err(format!("時間點最多 {MAX_TIMES} 個"));
        }
        for t in &spec.times {
            parse_time(t).ok_or_else(|| format!("時間格式不對（要 HH:MM）：{t}"))?;
        }
        Ok(())
    };
    match kind {
        KIND_DAILY => check_times(spec),
        KIND_WEEKLY => {
            check_times(spec)?;
            if spec.days.is_empty() || spec.days.iter().any(|d| !(1..=7).contains(d)) {
                return Err("每週模式要勾選星期（1–7）".into());
            }
            Ok(())
        }
        KIND_INTERVAL => {
            if spec.every_minutes < MIN_INTERVAL_MINUTES {
                return Err(format!("間隔不能小於 {MIN_INTERVAL_MINUTES} 分鐘"));
            }
            if let Some(w) = &spec.window {
                let (f, t) = (parse_time(&w.from), parse_time(&w.to));
                match (f, t) {
                    (Some(f), Some(t)) if f < t => {}
                    _ => return Err("時段要是 HH:MM，且起點早於終點".into()),
                }
            }
            Ok(())
        }
        other => Err(format!("不認得的週期模式：{other}")),
    }
}

/// 從 `last` 之後到 `now`（含）之間，是否有一個預定時間點已經過了。
/// `last` 是上次觸發（或建立／啟用）的本地時間。全是本地時間，沒有時區問題。
pub fn due(kind: &str, spec: &Spec, last: NaiveDateTime, now: NaiveDateTime) -> bool {
    if now <= last {
        return false;
    }
    match kind {
        KIND_INTERVAL => {
            if now < last + Duration::minutes(i64::from(spec.every_minutes)) {
                return false;
            }
            match &spec.window {
                None => true,
                Some(w) => match (parse_time(&w.from), parse_time(&w.to)) {
                    (Some(f), Some(t)) => now.time() >= f && now.time() <= t,
                    _ => false,
                },
            }
        }
        KIND_DAILY | KIND_WEEKLY => {
            let mut day: NaiveDate = last.date();
            let mut guard = 0;
            while day <= now.date() && guard < 400 {
                let weekday_ok = kind == KIND_DAILY || spec.days.contains(&day.weekday().number_from_monday());
                if weekday_ok {
                    for t in &spec.times {
                        if let Some(t) = parse_time(t) {
                            let slot = day.and_time(t);
                            if slot > last && slot <= now {
                                return true;
                            }
                        }
                    }
                }
                day = day.succ_opt().unwrap_or(now.date());
                if day == now.date() && day.succ_opt().is_none() {
                    break;
                }
                guard += 1;
            }
            false
        }
        _ => false,
    }
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

    #[test]
    fn daily_does_not_fire_one_minute_before_the_slot() {
        assert!(!due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-15 20:00"), dt("2026-11-16 07:59")));
    }

    #[test]
    fn daily_fires_at_and_after_the_slot() {
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-15 20:00"), dt("2026-11-16 08:00")));
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-15 20:00"), dt("2026-11-16 08:01")));
    }

    #[test]
    fn daily_does_not_fire_twice_for_the_same_slot() {
        // 08:00 的那一次已在 08:00 觸發過，之後的檢查不再成立。
        assert!(!due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-16 08:00"), dt("2026-11-16 08:01")));
        assert!(!due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-16 08:00"), dt("2026-11-16 23:59")));
    }

    #[test]
    fn daily_fires_again_next_day_and_after_a_gap_only_reports_due_once() {
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-16 08:00"), dt("2026-11-17 08:00")));
        // 隔了三天沒開 App：仍只是「該觸發」一次（呼叫端把 last 設成 now）。
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-16 08:00"), dt("2026-11-19 09:00")));
    }

    #[test]
    fn daily_multiple_times_fire_each() {
        let s = daily(&["08:00", "18:30"]);
        assert!(!due(KIND_DAILY, &s, dt("2026-11-16 08:00"), dt("2026-11-16 18:29")));
        assert!(due(KIND_DAILY, &s, dt("2026-11-16 08:00"), dt("2026-11-16 18:30")));
    }

    #[test]
    fn daily_crosses_month_boundary() {
        assert!(due(KIND_DAILY, &daily(&["08:00"]), dt("2026-11-30 08:00"), dt("2026-12-01 08:00")));
    }

    #[test]
    fn weekly_only_fires_on_selected_days() {
        // 2026-11-16 是週一。
        let s = Spec { times: vec!["09:00".into()], days: vec![1, 2, 3, 4, 5], ..Default::default() };
        assert!(due(KIND_WEEKLY, &s, dt("2026-11-14 09:00"), dt("2026-11-16 09:00"))); // 六 → 一
        assert!(!due(KIND_WEEKLY, &s, dt("2026-11-14 09:00"), dt("2026-11-15 12:00"))); // 週日不觸發
        let sat_only = Spec { times: vec!["09:00".into()], days: vec![6], ..Default::default() };
        assert!(!due(KIND_WEEKLY, &sat_only, dt("2026-11-16 09:00"), dt("2026-11-20 12:00")));
        assert!(due(KIND_WEEKLY, &sat_only, dt("2026-11-16 09:00"), dt("2026-11-21 09:00")));
    }

    #[test]
    fn interval_fires_after_the_period() {
        let s = Spec { every_minutes: 120, ..Default::default() };
        assert!(!due(KIND_INTERVAL, &s, dt("2026-11-16 08:00"), dt("2026-11-16 09:59")));
        assert!(due(KIND_INTERVAL, &s, dt("2026-11-16 08:00"), dt("2026-11-16 10:00")));
    }

    #[test]
    fn interval_respects_the_window() {
        let s = Spec { every_minutes: 60, window: Some(Window { from: "09:00".into(), to: "18:00".into() }), ..Default::default() };
        assert!(!due(KIND_INTERVAL, &s, dt("2026-11-16 17:00"), dt("2026-11-16 18:01"))); // 窗外
        assert!(due(KIND_INTERVAL, &s, dt("2026-11-16 17:00"), dt("2026-11-16 18:00")));
        // 夜裡到期，隔天進入時段才觸發。
        assert!(!due(KIND_INTERVAL, &s, dt("2026-11-16 17:30"), dt("2026-11-16 23:00")));
        assert!(due(KIND_INTERVAL, &s, dt("2026-11-16 17:30"), dt("2026-11-17 09:00")));
    }

    #[test]
    fn validate_rejects_bad_specs() {
        assert!(validate(KIND_DAILY, &Spec::default()).is_err());
        assert!(validate(KIND_DAILY, &daily(&["25:00"])).is_err());
        assert!(validate(KIND_DAILY, &daily(&["08:00"])).is_ok());
        let many: Vec<&str> = vec!["08:00"; 13];
        assert!(validate(KIND_DAILY, &daily(&many)).is_err());
        assert!(validate(KIND_WEEKLY, &daily(&["08:00"])).is_err()); // 沒勾星期
        assert!(validate(KIND_WEEKLY, &Spec { times: vec!["08:00".into()], days: vec![8], ..Default::default() }).is_err());
        assert!(validate(KIND_INTERVAL, &Spec { every_minutes: 4, ..Default::default() }).is_err());
        assert!(validate(KIND_INTERVAL, &Spec { every_minutes: 5, ..Default::default() }).is_ok());
        let bad_window = Spec { every_minutes: 60, window: Some(Window { from: "18:00".into(), to: "09:00".into() }), ..Default::default() };
        assert!(validate(KIND_INTERVAL, &bad_window).is_err());
        assert!(validate("monthly", &Spec::default()).is_err());
    }
}
```

- [ ] **Step 2: 執行確認有紅燈**

在 `tasks/mod.rs` 加 `pub mod schedules;`。Run: `cd src-tauri && cargo test --workspace --no-fail-fast schedules::tests`
Expected: 把 `due` 本體先留 `todo!()` 時全部 FAIL（panic not yet implemented）。

- [ ] **Step 3: 填入實作並確認通過**

上面程式碼已含 `validate`、`due` 實作；移除 `todo!()` 版本即可。Run 同上，Expected: 全部 PASS。接著做突變：把 `slot > last` 改成 `slot >= last`，`daily_does_not_fire_twice_for_the_same_slot` 必須變紅，還原。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/tasks/schedules.rs src-tauri/src/tasks/mod.rs
git commit -m "feat(schedules): 週期規格驗證與 due 判斷"
```

---

### Task 2: 資料表與存取

**Files:** Modify `src-tauri/src/tasks/schedules.rs`、`src-tauri/src/tasks/mod.rs`

- [ ] **Step 1: 寫失敗測試**（附在 `schedules.rs` 的 tests 模組，用 `SqlitePoolOptions::new().connect("sqlite::memory:")`，仿 `notes.rs` 的測試寫法）

```rust
async fn pool() -> sqlx::SqlitePool {
    let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    init_schema(&p).await.unwrap();
    p
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

#[tokio::test]
async fn create_then_list_roundtrips_the_spec() {
    let pool = pool().await;
    let id = create(&pool, &new_schedule("a1"), 1_000).await.unwrap();
    let rows = list(&pool).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].spec().unwrap(), daily(&["08:00"]));
    assert_eq!(rows[0].last_fired_at, 1_000, "建立時以現在當基準，避免一建立就因過了時間點而觸發");
    assert!(rows[0].enabled);
}

#[tokio::test]
async fn create_rejects_an_invalid_spec() {
    let pool = pool().await;
    let mut s = new_schedule("a1");
    s.spec = Spec::default();
    assert!(create(&pool, &s, 0).await.is_err());
}

#[tokio::test]
async fn enabling_resets_streaks_reason_and_baseline() {
    let pool = pool().await;
    let id = create(&pool, &new_schedule("a1"), 1_000).await.unwrap();
    sqlx::query("UPDATE schedules SET enabled = 0, fail_streak = 3, skip_streak = 2, disabled_reason = '連續失敗' WHERE id = ?")
        .bind(&id).execute(&pool).await.unwrap();
    set_enabled(&pool, &id, true, 5_000).await.unwrap();
    let r = get(&pool, &id).await.unwrap().unwrap();
    assert!(r.enabled && r.fail_streak == 0 && r.skip_streak == 0 && r.disabled_reason.is_none());
    assert_eq!(r.last_fired_at, 5_000);
}

#[tokio::test]
async fn runs_keep_only_the_latest_five() {
    let pool = pool().await;
    let id = create(&pool, &new_schedule("a1"), 0).await.unwrap();
    for i in 0..7 {
        record_run(&pool, &id, i, None, "success").await.unwrap();
    }
    let runs = list_runs(&pool, &id).await.unwrap();
    assert_eq!(runs.len(), 5);
    assert_eq!(runs[0].fired_at, 6, "新的在前");
}

#[tokio::test]
async fn delete_removes_runs_too() {
    let pool = pool().await;
    let id = create(&pool, &new_schedule("a1"), 0).await.unwrap();
    record_run(&pool, &id, 1, None, "success").await.unwrap();
    delete(&pool, &id).await.unwrap();
    assert!(list(&pool).await.unwrap().is_empty());
    assert!(list_runs(&pool, &id).await.unwrap().is_empty());
}
```

- [ ] **Step 2: 執行確認紅燈**（`create` 等尚未存在，編譯失敗即為紅燈）。

- [ ] **Step 3: 實作**（附在 `schedules.rs`，`due` 之後）

```rust
use sqlx::SqlitePool;

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

pub const MAX_RUNS_KEPT: i64 = 5;

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
    if t.is_empty() { Err(format!("{what}不能是空的")) } else { Ok(t.to_string()) }
}

/// `now`：建立當下的 unix 秒，當作 `last_fired_at` 的基準。
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
    .bind(&id).bind(name).bind(&s.agent_id).bind(instruction).bind(&s.schedule_kind)
    .bind(serde_json::to_string(&s.spec).map_err(|e| e.to_string())?)
    .bind(s.failures_count_as_failed).bind(s.canvas_x).bind(s.canvas_y).bind(now)
    .execute(pool).await.map_err(|e| e.to_string())?;
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
    .bind(name).bind(&s.agent_id).bind(instruction).bind(&s.schedule_kind)
    .bind(serde_json::to_string(&s.spec).map_err(|e| e.to_string())?)
    .bind(s.failures_count_as_failed).bind(s.canvas_x).bind(s.canvas_y).bind(id)
    .execute(pool).await.map_err(|e| e.to_string())?;
    if res.rows_affected() == 0 { return Err("找不到這個定時器".into()); }
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
        .bind(now).bind(id).execute(pool).await?;
    } else {
        sqlx::query("UPDATE schedules SET enabled = 0, disabled_reason = NULL WHERE id = ?")
            .bind(id).execute(pool).await?;
    }
    Ok(())
}

pub async fn record_run(pool: &SqlitePool, id: &str, fired_at: i64, module_id: Option<&str>, result: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO schedule_runs (schedule_id, fired_at, module_id, result) VALUES (?, ?, ?, ?)")
        .bind(id).bind(fired_at).bind(module_id).bind(result).execute(pool).await?;
    sqlx::query(
        "DELETE FROM schedule_runs WHERE schedule_id = ?1 AND rowid NOT IN
           (SELECT rowid FROM schedule_runs WHERE schedule_id = ?1 ORDER BY fired_at DESC, rowid DESC LIMIT ?2)",
    )
    .bind(id).bind(MAX_RUNS_KEPT).execute(pool).await?;
    Ok(())
}

pub async fn list_runs(pool: &SqlitePool, id: &str) -> Result<Vec<RunRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM schedule_runs WHERE schedule_id = ? ORDER BY fired_at DESC, rowid DESC")
        .bind(id).fetch_all(pool).await
}
```

另外在 `tasks/mod.rs` 的 `init_schema` 末尾（`fix_choice::init_schema(pool).await?;` 之後）加 `schedules::init_schema(pool).await?;`。

- [ ] **Step 4: 執行確認通過**；加一條「舊資料庫（只有 tasks 表）呼叫 `tasks::init_schema` 後 `schedules` 表存在」的測試放在 `tasks/mod.rs` 現有測試風格處，並確認紅→綠（拿掉 `init_schema` 呼叫時應失敗）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/schedules.rs src-tauri/src/tasks/mod.rs
git commit -m "feat(schedules): 定時器資料表與存取"
```

---

### Task 3: `fire`（建一輪）

**Files:** Modify `src-tauri/src/tasks/schedules.rs`；需要把 `workflow.rs` 的 `verify_body`、`verify_json_path`、`evidence_dir` 已是 `pub`（確認）；`insert_card`／`NewCard`／`queued_tail` 是私有，**在 `workflow.rs` 新增兩個 `pub` 包裝**（見 Step 3）。

- [ ] **Step 1: 寫失敗測試**（`schedules.rs` tests，測試用的 agent 以 `agents::create_agent` 建立，仿 `workflow.rs` 的測試 helper；若沒有可共用 helper，在 tests 內寫 `async fn coder(pool) -> String` 呼叫 `agents::create_agent` 並以 `kind: "coder"`）：

```rust
#[tokio::test]
async fn firing_for_a_coder_creates_a_queued_work_card_owned_by_it() {
    let pool = pool_with_tasks().await; // tasks::init_schema 後的記憶體庫
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
    let row = get(&pool, &sid).await.unwrap().unwrap();
    let outcome = fire(&pool, std::path::Path::new("/tmp/p"), &row, 10_000, None).await.unwrap();
    let Fired::Created { card_id } = outcome else { panic!("應建卡") };
    let card = store::get_task(&pool, &card_id).await.unwrap().unwrap();
    assert_eq!(card.card_kind, "work");
    assert_eq!(card.status, "queued");
    assert_eq!(card.agent_id.as_deref(), Some(coder.as_str()));
    assert!(card.module_id.is_none(), "模組本身 module_id 為空");
    assert!(card.body.contains("跑完整測試"), "內文帶固定指令");
    let after = get(&pool, &sid).await.unwrap().unwrap();
    assert_eq!(after.last_fired_at, 10_000);
    assert_eq!(after.last_module_id.as_deref(), Some(card_id.as_str()));
    assert_eq!(list_runs(&pool, &sid).await.unwrap().len(), 0, "成敗在 settle 時才記，建卡當下不記");
}

#[tokio::test]
async fn firing_for_a_verifier_creates_a_verify_card_pinned_to_head() {
    let pool = pool_with_tasks().await;
    let v = make_agent(&pool, "verifier").await;
    let sid = create(&pool, &new_schedule(&v), 0).await.unwrap();
    let row = get(&pool, &sid).await.unwrap().unwrap();
    let Fired::Created { card_id } = fire(&pool, std::path::Path::new("/tmp/p"), &row, 10_000, Some("abc123")).await.unwrap() else { panic!() };
    let card = store::get_task(&pool, &card_id).await.unwrap().unwrap();
    assert_eq!(card.card_kind, "verify");
    assert_eq!(card.end_sha.as_deref(), Some("abc123"), "沒有合併卡時受驗版本是目前 HEAD");
    assert!(card.review_of.is_none());
    assert!(card.body.contains("跑完整測試"));
}

#[tokio::test]
async fn firing_is_skipped_while_the_previous_round_is_open() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
    let row = get(&pool, &sid).await.unwrap().unwrap();
    let Fired::Created { .. } = fire(&pool, std::path::Path::new("/tmp/p"), &row, 10_000, None).await.unwrap() else { panic!() };
    let row = get(&pool, &sid).await.unwrap().unwrap();
    assert!(matches!(fire(&pool, std::path::Path::new("/tmp/p"), &row, 20_000, None).await.unwrap(), Fired::Skipped));
    let after = get(&pool, &sid).await.unwrap().unwrap();
    assert_eq!(after.skip_streak, 1);
    assert_eq!(after.last_fired_at, 20_000, "跳過也推進基準，否則每分鐘都再試一次");
    assert_eq!(list_runs(&pool, &sid).await.unwrap()[0].result, "skipped");
    assert_eq!(store::list_tasks(&pool).await.unwrap().len(), 1, "沒有多建卡");
}

#[tokio::test]
async fn a_missing_target_agent_creates_nothing_and_errors() {
    let pool = pool_with_tasks().await;
    let sid = create(&pool, &new_schedule("ghost"), 0).await.unwrap();
    let row = get(&pool, &sid).await.unwrap().unwrap();
    assert!(fire(&pool, std::path::Path::new("/tmp/p"), &row, 10_000, None).await.is_err());
    assert!(store::list_tasks(&pool).await.unwrap().is_empty());
    assert_eq!(get(&pool, &sid).await.unwrap().unwrap().last_fired_at, 0, "失敗不推進基準");
}
```

「上一輪還沒結案」的判定：`last_module_id` 指向的卡存在，且（work 卡）`module_state` 不是終點、（verify 卡）`status != done`。把它寫成 `async fn round_open(pool, row) -> Result<bool>`，與 `settle`（Task 4）共用 `round_result`。

- [ ] **Step 2: 執行確認紅燈**（`fire`、`Fired`、`make_agent`、`pool_with_tasks` 不存在）。

- [ ] **Step 3: 實作**

`workflow.rs` 新增（放在 `create_ask_card` 之後）：

```rust
/// 定時器建一個模組（work 卡，`module_id` 為空）給工程師，排在佇列最後。
pub async fn create_scheduled_work_card(pool: &SqlitePool, agent: &AgentRow, title: &str, body: &str) -> Result<String, sqlx::Error> {
    insert_card(
        pool,
        &NewCard {
            title, body, project_dir: &agent.project_dir, agent_id: &agent.id, card_kind: KIND_WORK,
            module_id: None, review_of: None, start_sha: None, end_sha: None,
            sort_order: queued_tail(pool).await?, from_spec: None,
        },
    )
    .await
}

/// 定時器直接建一張驗收卡（沒有合併卡）：受驗版本是呼叫端給的 `head_sha`，
/// 內文是標準驗收提示加上定時器的固定指令。
pub async fn create_scheduled_verify_card(
    pool: &SqlitePool, project_path: &Path, verifier: &AgentRow, title: &str, instruction: &str, head_sha: &str,
) -> Result<String, sqlx::Error> {
    let all_agents = agents::list_agents(pool).await?;
    let tasks = store::list_tasks(pool).await?;
    let modules = coder_modules(&all_agents, &tasks);
    let id = insert_card(
        pool,
        &NewCard {
            title, body: "", project_dir: &verifier.project_dir, agent_id: &verifier.id, card_kind: KIND_VERIFY,
            module_id: None, review_of: None, start_sha: None, end_sha: Some(head_sha),
            sort_order: queued_tail(pool).await?, from_spec: None,
        },
    )
    .await?;
    let mut body = verify_body(&modules, "", &verify_json_path(project_path, &id), &evidence_dir(project_path, &id));
    body.push_str(&format!("\n這是定時觸發的驗收。額外指示：\n{}\n", instruction.trim()));
    sqlx::query("UPDATE tasks SET body = ? WHERE id = ?").bind(&body).bind(&id).execute(pool).await?;
    Ok(id)
}
```

`schedules.rs`：

```rust
use crate::tasks::{agents, store, workflow};

#[derive(Debug, PartialEq)]
pub enum Fired {
    Created { card_id: String },
    Skipped,
}

pub fn card_title(name: &str) -> String {
    format!("定時：{name}")
}

fn work_body(instruction: &str) -> String {
    format!("這是定時觸發的工作（每次觸發都是一輪新的工作，請依下面的指示完成並照一般流程交付）：\n\n{}\n", instruction.trim())
}

/// 建一輪。`head_sha`：目標是驗收者時必填（受驗版本），其他角色忽略。
/// 上一輪還沒結案 → 跳過（記一筆 skipped、`skip_streak`+1），並照樣推進 `last_fired_at`。
pub async fn fire(pool: &SqlitePool, project_path: &std::path::Path, row: &ScheduleRow, now: i64, head_sha: Option<&str>) -> Result<Fired, String> {
    let agent = agents::get_agent(pool, &row.agent_id).await.map_err(|e| e.to_string())?
        .ok_or_else(|| "目標角色已不存在".to_string())?;
    if round_open(pool, row).await? {
        sqlx::query("UPDATE schedules SET skip_streak = skip_streak + 1, last_fired_at = ? WHERE id = ?")
            .bind(now).bind(&row.id).execute(pool).await.map_err(|e| e.to_string())?;
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
        .bind(now).bind(&card_id).bind(&row.id).execute(pool).await.map_err(|e| e.to_string())?;
    Ok(Fired::Created { card_id })
}
```

`round_open` 先放簡版（Task 4 補完整 `round_result`）：

```rust
pub async fn round_open(pool: &SqlitePool, row: &ScheduleRow) -> Result<bool, String> {
    Ok(matches!(round_result(pool, row).await?, RoundResult::Pending))
}
```

（`RoundResult`、`round_result` 在 Task 4 定義；本任務先以 `todo!()` 版本讓程式碼編譯，Task 4 補上。若要讓 Task 3 獨立通過，先寫成：只要 `last_module_id` 指向的卡存在且 `status != done` 就算進行中。Task 4 取代它。）

測試 helper（`tests` 內）：

```rust
async fn pool_with_tasks() -> SqlitePool {
    let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    crate::tasks::init_schema(&p).await.unwrap();
    p
}
async fn make_agent(pool: &SqlitePool, kind: &str) -> String {
    agents::create_agent(pool, &agents::NewAgent { kind, name: kind, role: kind, prompt_prefix: "", project_dir: "/tmp/repo", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: None })
        .await.unwrap()
}
```

（`NewAgent` 欄位以 `tasks/agents.rs:49` 的實際定義為準；若欄位不同，照實際欄位調整，保持測試意圖。）

- [ ] **Step 4: 執行確認通過**；突變：把 `round_open` 恆回 `false`，`firing_is_skipped…` 必須變紅，還原。

- [ ] **Step 5: Commit** `git commit -m "feat(schedules): fire 建一輪（工程師建模組、驗收者建驗收卡）並跳過未結案輪"`

---

### Task 4: `settle`（判定成敗、停用、歸檔）

**Files:** Modify `src-tauri/src/tasks/schedules.rs`

- [ ] **Step 1: 寫失敗測試**

```rust
async fn finish_card(pool: &SqlitePool, id: &str, outcome: &str, verdict: Option<&str>) {
    sqlx::query("UPDATE tasks SET status = 'done', outcome = ?, verdict = ? WHERE id = ?")
        .bind(outcome).bind(verdict).bind(id).execute(pool).await.unwrap();
}

#[tokio::test]
async fn a_coder_round_with_a_passed_module_succeeds_and_archives_its_chain() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
    let row = get(&pool, &sid).await.unwrap().unwrap();
    let Fired::Created { card_id } = fire(&pool, std::path::Path::new("/tmp/p"), &row, 10, None).await.unwrap() else { panic!() };
    // 沒有審核者：work 卡成功結案＝模組通過。
    finish_card(&pool, &card_id, "success", None).await;
    let events = settle(&pool, &sid, 20).await.unwrap();
    assert_eq!(events, vec![SettleEvent::Succeeded]);
    let after = get(&pool, &sid).await.unwrap().unwrap();
    assert_eq!(after.fail_streak, 0);
    assert!(after.last_module_id.is_none(), "這一輪已結案，下一輪可以建");
    assert!(store::get_task(&pool, &card_id).await.unwrap().unwrap().archived_at.is_some());
    assert_eq!(list_runs(&pool, &sid).await.unwrap()[0].result, "success");
}

#[tokio::test]
async fn a_failed_round_counts_and_keeps_the_cards_visible() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
    let row = get(&pool, &sid).await.unwrap().unwrap();
    let Fired::Created { card_id } = fire(&pool, std::path::Path::new("/tmp/p"), &row, 10, None).await.unwrap() else { panic!() };
    finish_card(&pool, &card_id, "failed", None).await;
    assert_eq!(settle(&pool, &sid, 20).await.unwrap(), vec![SettleEvent::Failed]);
    let after = get(&pool, &sid).await.unwrap().unwrap();
    assert_eq!(after.fail_streak, 1);
    assert!(store::get_task(&pool, &card_id).await.unwrap().unwrap().archived_at.is_none());
}

#[tokio::test]
async fn three_failures_in_a_row_disable_the_schedule() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
    let mut last = Vec::new();
    for i in 0..3 {
        let row = get(&pool, &sid).await.unwrap().unwrap();
        let Fired::Created { card_id } = fire(&pool, std::path::Path::new("/tmp/p"), &row, 10 + i, None).await.unwrap() else { panic!() };
        finish_card(&pool, &card_id, "failed", None).await;
        last = settle(&pool, &sid, 20 + i).await.unwrap();
    }
    assert_eq!(last, vec![SettleEvent::Failed, SettleEvent::AutoDisabled]);
    let after = get(&pool, &sid).await.unwrap().unwrap();
    assert!(!after.enabled);
    assert_eq!(after.disabled_reason.as_deref(), Some("連續失敗"));
}

#[tokio::test]
async fn a_success_resets_the_fail_streak() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
    sqlx::query("UPDATE schedules SET fail_streak = 2 WHERE id = ?").bind(&sid).execute(&pool).await.unwrap();
    let row = get(&pool, &sid).await.unwrap().unwrap();
    let Fired::Created { card_id } = fire(&pool, std::path::Path::new("/tmp/p"), &row, 10, None).await.unwrap() else { panic!() };
    finish_card(&pool, &card_id, "success", None).await;
    settle(&pool, &sid, 20).await.unwrap();
    assert_eq!(get(&pool, &sid).await.unwrap().unwrap().fail_streak, 0);
}

#[tokio::test]
async fn a_pending_round_settles_to_nothing() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
    let row = get(&pool, &sid).await.unwrap().unwrap();
    fire(&pool, std::path::Path::new("/tmp/p"), &row, 10, None).await.unwrap();
    assert!(settle(&pool, &sid, 20).await.unwrap().is_empty());
}

#[tokio::test]
async fn verify_round_respects_the_failures_switch() {
    for (count_as_failed, expect) in [(false, SettleEvent::Succeeded), (true, SettleEvent::Failed)] {
        let pool = pool_with_tasks().await;
        let v = make_agent(&pool, "verifier").await;
        let mut ns = new_schedule(&v);
        ns.failures_count_as_failed = count_as_failed;
        let sid = create(&pool, &ns, 0).await.unwrap();
        let row = get(&pool, &sid).await.unwrap().unwrap();
        let Fired::Created { card_id } = fire(&pool, std::path::Path::new("/tmp/p"), &row, 10, Some("abc")).await.unwrap() else { panic!() };
        finish_card(&pool, &card_id, "success", Some("fail")).await;
        assert_eq!(settle(&pool, &sid, 20).await.unwrap()[0], expect);
    }
}

#[tokio::test]
async fn a_deleted_round_card_just_closes_the_round() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), 0).await.unwrap();
    let row = get(&pool, &sid).await.unwrap().unwrap();
    let Fired::Created { card_id } = fire(&pool, std::path::Path::new("/tmp/p"), &row, 10, None).await.unwrap() else { panic!() };
    store::delete_task(&pool, &card_id).await.unwrap();
    assert!(settle(&pool, &sid, 20).await.unwrap().is_empty());
    let after = get(&pool, &sid).await.unwrap().unwrap();
    assert!(after.last_module_id.is_none());
    assert_eq!(after.fail_streak, 0);
}
```

- [ ] **Step 2: 執行確認紅燈**（`settle`、`SettleEvent` 不存在）。

- [ ] **Step 3: 實作**

```rust
pub const MAX_FAIL_STREAK: i64 = 3;
pub const STUCK_SKIP_STREAK: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundResult { Pending, Success, Failed, Gone }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SettleEvent { Succeeded, Failed, AutoDisabled }

/// 上一輪現在的狀態。`last_module_id` 為空＝沒有進行中的輪，視為已結案（Gone）。
pub async fn round_result(pool: &SqlitePool, row: &ScheduleRow) -> Result<RoundResult, String> {
    let Some(id) = row.last_module_id.as_deref() else { return Ok(RoundResult::Gone) };
    let Some(card) = store::get_task(pool, id).await.map_err(|e| e.to_string())? else { return Ok(RoundResult::Gone) };
    if card.card_kind == workflow::KIND_VERIFY {
        if card.status != store::STATUS_DONE { return Ok(RoundResult::Pending); }
        if card.outcome.as_deref() != Some("success") { return Ok(RoundResult::Failed); }
        return Ok(match card.verdict.as_deref() {
            Some("pass") => RoundResult::Success,
            Some("fail") | Some("attention") if row.failures_count_as_failed => RoundResult::Failed,
            _ => RoundResult::Success,
        });
    }
    // 工程師起點：看模組鏈。
    let tasks = store::list_tasks(pool).await.map_err(|e| e.to_string())?;
    let chain: Vec<store::TaskRow> = tasks.iter().filter(|t| t.module_id.as_deref() == Some(card.id.as_str())).cloned().collect();
    let has_reviewer = agents::list_agents(pool).await.map_err(|e| e.to_string())?.iter().any(|a| a.kind == "reviewer");
    use workflow::ModuleState as M;
    Ok(match workflow::module_state(&card, &chain, has_reviewer) {
        M::Passed => RoundResult::Success,
        M::Attention | M::Failed | M::ReviewFailed | M::DispatchFailed => RoundResult::Failed,
        _ => RoundResult::Pending,
    })
}

/// 判定一輪並更新狀態。回傳這次發生的事（空＝還在進行或沒有進行中的輪）。
pub async fn settle(pool: &SqlitePool, id: &str, now: i64) -> Result<Vec<SettleEvent>, String> {
    let Some(row) = get(pool, id).await.map_err(|e| e.to_string())? else { return Ok(vec![]) };
    let module_id = row.last_module_id.clone();
    let result = round_result(pool, &row).await?;
    let mut events = Vec::new();
    match result {
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
                // 模組本身＋整條鏈。只歸檔已完成的（歸檔是看板上的可見性層，不碰進行中的卡）。
                sqlx::query("UPDATE tasks SET archived_at = ? WHERE (id = ?1 OR module_id = ?1) AND status = 'done' AND archived_at IS NULL")
                    .bind(m).execute(pool).await.map_err(|e| e.to_string())?;
            }
            events.push(SettleEvent::Succeeded);
        }
        RoundResult::Failed => {
            let streak = row.fail_streak + 1;
            sqlx::query("UPDATE schedules SET fail_streak = ?, last_module_id = NULL WHERE id = ?").bind(streak).bind(id).execute(pool).await.map_err(|e| e.to_string())?;
            record_run(pool, id, now, module_id.as_deref(), "failed").await.map_err(|e| e.to_string())?;
            events.push(SettleEvent::Failed);
            if streak >= MAX_FAIL_STREAK {
                sqlx::query("UPDATE schedules SET enabled = 0, disabled_reason = '連續失敗' WHERE id = ?").bind(id).execute(pool).await.map_err(|e| e.to_string())?;
                events.push(SettleEvent::AutoDisabled);
            }
        }
    }
    Ok(events)
}
```

注意 `archived_at = ?` 綁的是 `now_secs()` 風格的 unix 秒：上面 SQL 的第一個 `?` 要綁 `now`，第二個佔位符用 `?1` 會與前者衝突——**改寫成命名一致的編號佔位符**：`"UPDATE tasks SET archived_at = ?1 WHERE (id = ?2 OR module_id = ?2) AND status = 'done' AND archived_at IS NULL"`，`.bind(now).bind(m)`。同理檢查 `record_run` 的 SQL 已用 `?1/?2`。

`round_open` 改成呼叫 `round_result`（Task 3 的簡版刪除）：`Pending` 才是進行中。

驗證 `store::TaskRow` 有 `outcome`、`verdict`、`archived_at` 欄位（`outcome` 在 `workflow.rs` 已被使用，`archived_at` 在 `store.rs:41`），且 `store::STATUS_DONE` 為 `pub`。

- [ ] **Step 4: 執行確認通過**；突變：把 `MAX_FAIL_STREAK` 改成 4，`three_failures…` 必須變紅；把歸檔 SQL 的 `status = 'done'` 拿掉，應在「進行中的卡也被歸檔」的測試（見 Step 1 可加一條：鏈上有一張 queued 的修正卡時不歸檔它）變紅——補這條測試。

- [ ] **Step 5: Commit** `git commit -m "feat(schedules): settle 判定成敗、連續失敗停用、成功自動歸檔"`

---

### Task 5: `tick` 與排程迴圈

**Files:** Modify `schedules.rs`、`src-tauri/src/tasks/scheduler.rs`

- [ ] **Step 1: 寫失敗測試**（`schedules.rs`）

```rust
fn at(s: &str) -> chrono::NaiveDateTime { chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap() }

#[tokio::test]
async fn tick_fires_a_due_schedule_exactly_once() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
    let ctx = TickCtx { project_path: std::path::PathBuf::from("/tmp/p"), head_sha: None };
    let r1 = tick(&pool, &ctx, at("2026-11-16 07:59")).await.unwrap();
    assert!(r1.fired.is_empty());
    let r2 = tick(&pool, &ctx, at("2026-11-16 08:00")).await.unwrap();
    assert_eq!(r2.fired, vec![sid.clone()]);
    let r3 = tick(&pool, &ctx, at("2026-11-16 08:01")).await.unwrap();
    assert!(r3.fired.is_empty(), "同一個時間點不重複");
    assert_eq!(store::list_tasks(&pool).await.unwrap().len(), 1);
}

#[tokio::test]
async fn tick_ignores_disabled_schedules() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
    set_enabled_raw(&pool, &sid, false).await;
    let ctx = TickCtx { project_path: std::path::PathBuf::from("/tmp/p"), head_sha: None };
    assert!(tick(&pool, &ctx, at("2026-11-16 09:00")).await.unwrap().fired.is_empty());
}

#[tokio::test]
async fn tick_disables_schedules_whose_agent_is_gone() {
    let pool = pool_with_tasks().await;
    let sid = create(&pool, &new_schedule("ghost"), local_ts(at("2026-11-15 20:00"))).await.unwrap();
    let ctx = TickCtx { project_path: std::path::PathBuf::from("/tmp/p"), head_sha: None };
    tick(&pool, &ctx, at("2026-11-16 09:00")).await.unwrap();
    let r = get(&pool, &sid).await.unwrap().unwrap();
    assert!(!r.enabled);
    assert_eq!(r.disabled_reason.as_deref(), Some("角色已不存在"));
}

#[tokio::test]
async fn tick_settles_finished_rounds_before_firing_the_next() {
    let pool = pool_with_tasks().await;
    let coder = make_agent(&pool, "coder").await;
    let sid = create(&pool, &new_schedule(&coder), local_ts(at("2026-11-15 20:00"))).await.unwrap();
    let ctx = TickCtx { project_path: std::path::PathBuf::from("/tmp/p"), head_sha: None };
    tick(&pool, &ctx, at("2026-11-16 08:00")).await.unwrap();
    let card = get(&pool, &sid).await.unwrap().unwrap().last_module_id.unwrap();
    finish_card(&pool, &card, "success", None).await;
    let r = tick(&pool, &ctx, at("2026-11-17 08:00")).await.unwrap();
    assert!(r.events.iter().any(|(_, e)| *e == SettleEvent::Succeeded));
    assert_eq!(r.fired, vec![sid], "前一輪已結案，所以今天這輪能建，不會被跳過");
}
```

需要 `local_ts(NaiveDateTime) -> i64`（以本地時區換 unix 秒）與反向 `from_ts(i64) -> NaiveDateTime`（本地）。**測試要用與實作相同的換算，才不受執行機器時區影響**：兩者放在 `schedules.rs` 的 `pub fn`，用 `chrono::Local`。`set_enabled_raw` 是 tests 內直接 `UPDATE schedules SET enabled = 0`。

- [ ] **Step 2: 執行確認紅燈**。

- [ ] **Step 3: 實作**

```rust
use chrono::{Local, TimeZone};

pub fn local_ts(dt: NaiveDateTime) -> i64 {
    Local.from_local_datetime(&dt).earliest().map(|d| d.timestamp()).unwrap_or_else(|| dt.and_utc().timestamp())
}
pub fn from_ts(ts: i64) -> NaiveDateTime {
    Local.timestamp_opt(ts, 0).single().map(|d| d.naive_local()).unwrap_or_default()
}

pub struct TickCtx {
    pub project_path: std::path::PathBuf,
    /// 驗收者起點需要的受驗版本；由呼叫端（有 repo 的地方）算好。沒有就不能叫起驗收者。
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
        if !row.enabled { continue }
        if agents::get_agent(pool, &row.agent_id).await.map_err(|e| e.to_string())?.is_none() {
            sqlx::query("UPDATE schedules SET enabled = 0, disabled_reason = '角色已不存在' WHERE id = ?").bind(&row.id).execute(pool).await.map_err(|e| e.to_string())?;
            continue;
        }
        let spec = match row.spec() { Ok(s) => s, Err(_) => continue };
        if !due(&row.schedule_kind, &spec, from_ts(row.last_fired_at), now) { continue }
        match fire(pool, &ctx.project_path, &row, now_ts, ctx.head_sha.as_deref()).await {
            Ok(Fired::Created { .. }) => report.fired.push(row.id.clone()),
            Ok(Fired::Skipped) => {}
            Err(e) => eprintln!("schedule {} fire: {e}", row.name),
        }
    }
    Ok(report)
}
```

`scheduler.rs`：在 `spawn` 的 `tauri::async_runtime::spawn(async move { ... })` 內、`app.manage(quota.clone());` 之後加：

```rust
        spawn_schedule_loop(app.clone(), handle_for_schedules);
```

其中 `handle_for_schedules` 是 `SchedulerHandle` 的 clone（`spawn` 函式開頭已建 `handle`，在 move 進閉包前 `let sched_handle = handle.clone();`——`SchedulerHandle` 欄位皆為 `Arc`，需確認它 `derive(Clone)`，沒有就加）。新函式（同檔，`spawn_question_reminders` 附近）：

```rust
/// 每分鐘掃一次所有已開啟專案的定時器。建了新卡就叫醒派工並通知畫面重讀。
fn spawn_schedule_loop(app: AppHandle, sched: SchedulerHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let reg = app.state::<ProjectRegistry>();
            for project in reg.all() {
                // 驗收者起點需要受驗版本：用該專案任一驗收者的 repo 的 HEAD。
                let head_sha = schedule_head_sha(&project.pool).await;
                let ctx = crate::tasks::schedules::TickCtx { project_path: project.path.clone(), head_sha };
                match crate::tasks::schedules::tick(&project.pool, &ctx, chrono::Local::now().naive_local()).await {
                    Ok(r) if !r.fired.is_empty() || !r.events.is_empty() => {
                        for (sid, ev) in &r.events {
                            if *ev == crate::tasks::schedules::SettleEvent::AutoDisabled {
                                let name = crate::tasks::schedules::get(&project.pool, sid).await.ok().flatten().map(|s| s.name).unwrap_or_default();
                                let _ = app.emit(
                                    "agent-reply-wait",
                                    serde_json::json!({ "title": format!("定時器「{name}」已因連續失敗停用"), "project_name": project.name }),
                                );
                            }
                        }
                        sched.poke();
                        let _ = app.emit("tasks-updated", ());
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("schedule tick ({}): {e}", project.name),
                }
            }
        }
    });
}

async fn schedule_head_sha(pool: &sqlx::SqlitePool) -> Option<String> {
    let agents = crate::tasks::agents::list_agents(pool).await.ok()?;
    let verifier = agents.iter().find(|a| a.kind == "verifier")?;
    crate::vcs::git::GitClient::new(verifier.project_dir.clone(), None).head_sha().await.ok()
}
```

用 `grep -n "derive.*Clone" scheduler.rs` 確認 `SchedulerHandle`；`reg.all()` 已在 `spawn` 內被使用。

- [ ] **Step 4: 執行確認通過**（`cargo test --workspace --no-fail-fast schedules`），再 `cargo check` 確認 scheduler 改動編譯。突變：把 `due` 呼叫拿掉改成恆真，`tick_fires… exactly_once` 必須變紅。

- [ ] **Step 5: Commit** `git commit -m "feat(schedules): tick 結算並觸發，排程迴圈每分鐘掃描"`

---

### Task 6: Tauri 指令

**Files:** Create `src-tauri/src/commands/schedules.rs`；Modify `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs`

- [ ] **Step 1: 實作指令**（薄委派，仿 `commands/agents.rs`；`project`、`emit_updated` 從 `super::tasks` 取，簽章先 `grep -n "pub fn project\|pub fn emit_updated" src-tauri/src/commands/tasks.rs` 確認）：

```rust
//! 定時器的 Tauri 指令：薄薄一層委派給 `tasks::schedules`。
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::projects::ProjectRegistry;
use crate::tasks::schedules::{self, NewSchedule, RunRow, ScheduleRow};
use crate::tasks::scheduler::SchedulerHandle;

use super::tasks::{emit_updated, project};

#[derive(Serialize)]
pub struct ScheduleView {
    #[serde(flatten)]
    pub row: ScheduleRow,
    pub runs: Vec<RunRow>,
}

fn now_secs() -> i64 { chrono::Utc::now().timestamp() }

#[tauri::command]
pub async fn schedules_list(project_id: String, reg: State<'_, ProjectRegistry>) -> Result<Vec<ScheduleView>, String> {
    let p = project(&reg, &project_id)?;
    let mut out = Vec::new();
    for row in schedules::list(&p.pool).await.map_err(|e| e.to_string())? {
        let runs = schedules::list_runs(&p.pool, &row.id).await.map_err(|e| e.to_string())?;
        out.push(ScheduleView { row, runs });
    }
    Ok(out)
}

#[tauri::command]
pub async fn schedules_save(project_id: String, id: Option<String>, args: NewSchedule, reg: State<'_, ProjectRegistry>, app: AppHandle) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let agent = crate::tasks::agents::get_agent(&p.pool, &args.agent_id).await.map_err(|e| e.to_string())?
        .ok_or_else(|| "找不到這個角色".to_string())?;
    if agent.kind != "coder" && agent.kind != "verifier" {
        return Err("定時器只能叫起工程師或驗收者".into());
    }
    let id = match id {
        Some(id) => { schedules::update(&p.pool, &id, &args).await?; id }
        None => schedules::create(&p.pool, &args, now_secs()).await?,
    };
    emit_updated(&app);
    Ok(id)
}

#[tauri::command]
pub async fn schedules_delete(project_id: String, id: String, reg: State<'_, ProjectRegistry>, app: AppHandle) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    schedules::delete(&p.pool, &id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

#[tauri::command]
pub async fn schedules_set_enabled(project_id: String, id: String, enabled: bool, reg: State<'_, ProjectRegistry>, app: AppHandle) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    schedules::set_enabled(&p.pool, &id, enabled, now_secs()).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

/// 立即執行一次：不更新基準時間以外的排程狀態（走 `fire` 的同一條路，所以上一輪未結案會被擋下並記為跳過）。
#[tauri::command]
pub async fn schedules_run_now(project_id: String, id: String, reg: State<'_, ProjectRegistry>, sched: State<'_, SchedulerHandle>, app: AppHandle) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let row = schedules::get(&p.pool, &id).await.map_err(|e| e.to_string())?.ok_or_else(|| "找不到這個定時器".to_string())?;
    let agent = crate::tasks::agents::get_agent(&p.pool, &row.agent_id).await.map_err(|e| e.to_string())?
        .ok_or_else(|| "目標角色已不存在".to_string())?;
    let head = if agent.kind == "verifier" {
        Some(crate::vcs::git::GitClient::new(agent.project_dir.clone(), None).head_sha().await.map_err(|e| format!("讀不到目前版本：{e}"))?)
    } else { None };
    // 立即執行不應吃掉下一個預定時間點：fire 會把 last_fired_at 設成現在，這裡還原。
    let before = row.last_fired_at;
    let fired = schedules::fire(&p.pool, &p.path, &row, now_secs(), head.as_deref()).await?;
    sqlx::query("UPDATE schedules SET last_fired_at = ? WHERE id = ?").bind(before).bind(&id).execute(&p.pool).await.map_err(|e| e.to_string())?;
    sched.poke();
    emit_updated(&app);
    Ok(match fired { schedules::Fired::Created { card_id } => card_id, schedules::Fired::Skipped => String::new() })
}
```

`commands/mod.rs` 加 `pub mod schedules;`（比照其他模組寫法）；`lib.rs` 兩處註冊（`use` 清單，約 144 行附近；`invoke_handler` 清單，約 820 行附近）加上 `schedules_list, schedules_save, schedules_delete, schedules_set_enabled, schedules_run_now`。`SchedulerHandle` 是否已 `.manage()` 以 `grep -n "State<'_, SchedulerHandle>" src-tauri/src/commands/agents.rs` 確認（`agents.rs` 已引入同型別，沿用其取得方式）。

- [ ] **Step 2: 編譯與測試**

Run: `cd src-tauri && cargo check --workspace && cargo test --workspace --no-fail-fast schedules`
Expected: 編譯通過、schedules 測試全綠。

- [ ] **Step 3: Commit** `git commit -m "feat(schedules): Tauri 指令（列出／儲存／刪除／啟停／立即執行）"`

---

### Task 7: 前端 IPC 與週期摘要

**Files:** Create `src/ipc/schedules.ts`、`src/ipc/schedules.test.ts`、`src/components/AgentCanvas/scheduleSummary.ts`、`scheduleSummary.test.ts`；Modify `src/lib/i18n.ts`

- [ ] **Step 1: 寫失敗測試**

`src/ipc/schedules.test.ts`（仿 `agents.test.ts` 的 mock invoke 寫法）：

```ts
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { deleteSchedule, listSchedules, runScheduleNow, saveSchedule, setScheduleEnabled } from "./schedules";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => vi.mocked(invoke).mockReset());

describe("schedules ipc", () => {
  it("listSchedules 呼叫 schedules_list", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await listSchedules("p1");
    expect(invoke).toHaveBeenCalledWith("schedules_list", { projectId: "p1" });
  });
  it("saveSchedule 沒有 id 就是新增", async () => {
    vi.mocked(invoke).mockResolvedValue("s1");
    const args = { name: "n", agent_id: "a", instruction: "i", schedule_kind: "daily" as const, spec: { times: ["08:00"] }, failures_count_as_failed: false };
    await saveSchedule("p1", null, args);
    expect(invoke).toHaveBeenCalledWith("schedules_save", { projectId: "p1", id: null, args });
  });
  it("deleteSchedule／setScheduleEnabled／runScheduleNow 帶對參數", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await deleteSchedule("p1", "s1");
    expect(invoke).toHaveBeenLastCalledWith("schedules_delete", { projectId: "p1", id: "s1" });
    await setScheduleEnabled("p1", "s1", true);
    expect(invoke).toHaveBeenLastCalledWith("schedules_set_enabled", { projectId: "p1", id: "s1", enabled: true });
    await runScheduleNow("p1", "s1");
    expect(invoke).toHaveBeenLastCalledWith("schedules_run_now", { projectId: "p1", id: "s1" });
  });
});
```

`scheduleSummary.test.ts`：

```ts
import { describe, expect, it } from "vitest";

import { translations } from "../../lib/i18n";
import { summarizeSchedule } from "./scheduleSummary";

const t = translations["zh-TW"];

describe("summarizeSchedule", () => {
  it("每天多個時間點", () => {
    expect(summarizeSchedule(t, "daily", { times: ["08:00", "18:30"] })).toBe("每天 08:00、18:30");
  });
  it("每週平日", () => {
    expect(summarizeSchedule(t, "weekly", { times: ["09:00"], days: [1, 2, 3, 4, 5] })).toBe("週一、週二、週三、週四、週五 09:00");
  });
  it("間隔：分鐘與小時", () => {
    expect(summarizeSchedule(t, "interval", { every_minutes: 30 })).toBe("每 30 分鐘");
    expect(summarizeSchedule(t, "interval", { every_minutes: 120 })).toBe("每 2 小時");
  });
  it("間隔帶時段", () => {
    expect(summarizeSchedule(t, "interval", { every_minutes: 120, window: { from: "09:00", to: "18:00" } })).toBe("每 2 小時（09:00–18:00）");
  });
});
```

- [ ] **Step 2: 執行確認紅燈**（模組不存在）。

- [ ] **Step 3: 實作**

`src/ipc/schedules.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";

export type ScheduleKind = "daily" | "weekly" | "interval";

export interface ScheduleSpec {
  times?: string[];
  /** 1=週一 … 7=週日 */
  days?: number[];
  every_minutes?: number;
  window?: { from: string; to: string } | null;
}

export interface ScheduleRun {
  schedule_id: string;
  fired_at: number;
  module_id: string | null;
  result: "success" | "failed" | "skipped";
}

/** 鏡射 Rust `ScheduleView`（ScheduleRow 攤平 ＋ runs）。 */
export interface ScheduleView {
  id: string;
  name: string;
  agent_id: string;
  instruction: string;
  schedule_kind: ScheduleKind;
  schedule_json: string;
  enabled: boolean;
  failures_count_as_failed: boolean;
  canvas_x: number | null;
  canvas_y: number | null;
  last_fired_at: number;
  last_module_id: string | null;
  fail_streak: number;
  skip_streak: number;
  disabled_reason: string | null;
  runs: ScheduleRun[];
}

export interface NewSchedule {
  name: string;
  agent_id: string;
  instruction: string;
  schedule_kind: ScheduleKind;
  spec: ScheduleSpec;
  failures_count_as_failed: boolean;
  canvas_x?: number | null;
  canvas_y?: number | null;
}

export const listSchedules = (projectId: string): Promise<ScheduleView[]> => invoke("schedules_list", { projectId });
export const saveSchedule = (projectId: string, id: string | null, args: NewSchedule): Promise<string> =>
  invoke("schedules_save", { projectId, id, args });
export const deleteSchedule = (projectId: string, id: string): Promise<void> => invoke("schedules_delete", { projectId, id });
export const setScheduleEnabled = (projectId: string, id: string, enabled: boolean): Promise<void> =>
  invoke("schedules_set_enabled", { projectId, id, enabled });
export const runScheduleNow = (projectId: string, id: string): Promise<string> => invoke("schedules_run_now", { projectId, id });

export function parseSpec(json: string): ScheduleSpec {
  try {
    return JSON.parse(json) as ScheduleSpec;
  } catch {
    return {};
  }
}
```

`scheduleSummary.ts`：

```ts
import type { ScheduleKind, ScheduleSpec } from "../../ipc/schedules";
import type { translations } from "../../lib/i18n";

type T = (typeof translations)["zh-TW"];

export function summarizeSchedule(t: T, kind: ScheduleKind, spec: ScheduleSpec): string {
  const times = (spec.times ?? []).join("、");
  if (kind === "daily") return t.sched_sum_daily(times);
  if (kind === "weekly") {
    const days = (spec.days ?? []).map((d) => t.sched_weekdays[d - 1] ?? String(d)).join("、");
    return `${days} ${spec.times?.join("、") ?? ""}`.trim();
  }
  const m = spec.every_minutes ?? 0;
  const every = m % 60 === 0 ? t.sched_sum_every_hours(m / 60) : t.sched_sum_every_minutes(m);
  return spec.window ? `${every}（${spec.window.from}–${spec.window.to}）` : every;
}
```

i18n 字串（zh-TW 區塊與 en 區塊都加，**en 要同步**；放在 `shell_badge_*` 附近，鍵名以 `sched_` 開頭；先 `grep -n "sched_" src/lib/i18n.ts` 確認無衝突）：

```ts
// zh-TW
sched_weekdays: ["週一", "週二", "週三", "週四", "週五", "週六", "週日"],
sched_sum_daily: (times: string) => `每天 ${times}`,
sched_sum_every_hours: (n: number) => `每 ${n} 小時`,
sched_sum_every_minutes: (n: number) => `每 ${n} 分鐘`,
sched_title: "定時器",
sched_add: "新增定時器",
sched_name: "名稱",
sched_agent: "叫起的角色",
sched_instruction: "固定指令",
sched_instruction_hint: "每次觸發都會把這段交給角色，再依序走審核與驗收。",
sched_mode: "週期",
sched_mode_daily: "每天",
sched_mode_weekly: "每週",
sched_mode_interval: "固定間隔",
sched_times: "時間點",
sched_add_time: "新增時間點",
sched_days: "星期",
sched_every: "每隔",
sched_minutes: "分鐘",
sched_window: "只在時段內",
sched_failures_fail: "驗收有失敗項目也算失敗",
sched_enabled: "啟用",
sched_run_now: "立即執行一次",
sched_runs: "最近 5 輪",
sched_run_success: "成功",
sched_run_failed: "失敗",
sched_run_skipped: "已跳過",
sched_no_runs: "還沒有執行紀錄",
sched_save: "儲存",
sched_delete: "刪除",
sched_disabled_failures: "已因連續失敗停用",
sched_disabled_agent_gone: "角色已不存在，已停用",
sched_stuck: "上一輪卡住了",
sched_status_enabled: "啟用中",
sched_status_disabled: "已停用",
sched_run_now_started: "已建立一輪工作",
sched_run_now_skipped: "上一輪還沒結案，這次沒有建立",
// en
sched_weekdays: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
sched_sum_daily: (times: string) => `Daily ${times}`,
sched_sum_every_hours: (n: number) => `Every ${n} h`,
sched_sum_every_minutes: (n: number) => `Every ${n} min`,
sched_title: "Scheduler",
sched_add: "Add scheduler",
sched_name: "Name",
sched_agent: "Role to run",
sched_instruction: "Fixed instruction",
sched_instruction_hint: "Handed to the role every time it fires, then goes through review and verification.",
sched_mode: "Repeat",
sched_mode_daily: "Daily",
sched_mode_weekly: "Weekly",
sched_mode_interval: "Interval",
sched_times: "Times",
sched_add_time: "Add a time",
sched_days: "Days",
sched_every: "Every",
sched_minutes: "minutes",
sched_window: "Only within a time window",
sched_failures_fail: "Count verification failures as a failed run",
sched_enabled: "Enabled",
sched_run_now: "Run once now",
sched_runs: "Last 5 runs",
sched_run_success: "Success",
sched_run_failed: "Failed",
sched_run_skipped: "Skipped",
sched_no_runs: "No runs yet",
sched_save: "Save",
sched_delete: "Delete",
sched_disabled_failures: "Disabled after repeated failures",
sched_disabled_agent_gone: "Role no longer exists; disabled",
sched_stuck: "Previous run is stuck",
sched_status_enabled: "Enabled",
sched_status_disabled: "Disabled",
sched_run_now_started: "Started a run",
sched_run_now_skipped: "Previous run is still open; nothing was created",
```

en 的 `scheduleSummary` 週間組字沿用 `sched_weekdays`（en 為 Mon…，摘要為 `Mon、Tue … 09:00`，可接受；`scheduleSummary.test.ts` 只驗 zh-TW）。

- [ ] **Step 4: 執行確認通過**：`npx vitest run src/ipc/schedules.test.ts src/components/AgentCanvas/scheduleSummary.test.ts src/lib/i18n.test.ts`；`npx tsc -b`。

- [ ] **Step 5: Commit** `git commit -m "feat(schedules): 前端 IPC、週期摘要與 i18n 字串"`

---

### Task 8: `ScheduleDialog`

**Files:** Create `src/components/AgentCanvas/ScheduleDialog.tsx`、`ScheduleDialog.test.tsx`；Modify `src/components/AgentCanvas/index.css`

- [ ] **Step 1: 寫失敗測試**（`@testing-library/react`＋`LocaleProvider`，仿 `ShellWarningBadge/index.test.tsx` 的 mount 寫法）

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import { ScheduleDialog } from "./ScheduleDialog";

const agent = (id: string, kind: AgentRow["kind"], name: string) => ({ id, kind, name }) as AgentRow;
const agents = [agent("c1", "coder", "工程師A"), agent("r1", "reviewer", "審核者"), agent("v1", "verifier", "驗收者")];

function mount(props: Partial<React.ComponentProps<typeof ScheduleDialog>> = {}) {
  const onSave = vi.fn().mockResolvedValue(undefined);
  render(
    <LocaleProvider>
      <ScheduleDialog agents={agents} initial={null} onSave={onSave} onClose={() => {}} {...props} />
    </LocaleProvider>,
  );
  return { onSave };
}

describe("ScheduleDialog", () => {
  it("角色下拉只列工程師與驗收者", () => {
    mount();
    const options = screen.getAllByRole("option").map((o) => o.textContent);
    expect(options).toContain("工程師A");
    expect(options).toContain("驗收者");
    expect(options).not.toContain("審核者");
  });

  it("名稱與指令都空白時不能儲存", async () => {
    const { onSave } = mount();
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).not.toHaveBeenCalled();
  });

  it("每天模式：儲存時送出時間點", async () => {
    const { onSave } = mount();
    await userEvent.type(screen.getByLabelText("名稱"), "每日測試");
    await userEvent.type(screen.getByLabelText("固定指令"), "跑完整測試");
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ name: "每日測試", agent_id: "c1", instruction: "跑完整測試", schedule_kind: "daily", spec: { times: ["08:00"] } }),
    );
  });

  it("切到每週才出現星期勾選，切到間隔才出現分鐘欄位", async () => {
    mount();
    expect(screen.queryByText("星期")).toBeNull();
    await userEvent.selectOptions(screen.getByLabelText("週期"), "weekly");
    expect(screen.getByText("星期")).toBeTruthy();
    await userEvent.selectOptions(screen.getByLabelText("週期"), "interval");
    expect(screen.getByLabelText("分鐘")).toBeTruthy();
    expect(screen.queryByText("星期")).toBeNull();
  });

  it("間隔小於 5 分鐘不能儲存", async () => {
    const { onSave } = mount();
    await userEvent.type(screen.getByLabelText("名稱"), "n");
    await userEvent.type(screen.getByLabelText("固定指令"), "i");
    await userEvent.selectOptions(screen.getByLabelText("週期"), "interval");
    const mins = screen.getByLabelText("分鐘");
    await userEvent.clear(mins);
    await userEvent.type(mins, "4");
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).not.toHaveBeenCalled();
  });

  it("編輯既有定時器：帶入原值，且有「立即執行一次」", async () => {
    const onRunNow = vi.fn().mockResolvedValue("");
    mount({
      initial: {
        id: "s1", name: "舊的", agent_id: "v1", instruction: "舊指令", schedule_kind: "daily", schedule_json: '{"times":["09:30"]}',
        enabled: true, failures_count_as_failed: false, canvas_x: null, canvas_y: null, last_fired_at: 0, last_module_id: null,
        fail_streak: 0, skip_streak: 0, disabled_reason: null, runs: [],
      },
      onRunNow,
    });
    expect((screen.getByLabelText("名稱") as HTMLInputElement).value).toBe("舊的");
    await userEvent.click(screen.getByRole("button", { name: "立即執行一次" }));
    expect(onRunNow).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 執行確認紅燈**。

- [ ] **Step 3: 實作**（版型沿用 `ReferencesDialog` 的 `task-dialog-backdrop`／`task-dialog` 與既有表單 class；以下為完整元件）：

```tsx
import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import { parseSpec, type NewSchedule, type ScheduleKind, type ScheduleSpec, type ScheduleView } from "../../ipc/schedules";

const MIN_INTERVAL = 5;
const MAX_TIMES = 12;

export function ScheduleDialog({
  agents,
  initial,
  onSave,
  onDelete,
  onRunNow,
  onClose,
}: {
  agents: AgentRow[];
  initial: ScheduleView | null;
  onSave: (s: NewSchedule) => Promise<void>;
  onDelete?: () => Promise<void>;
  onRunNow?: () => Promise<string>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const targets = agents.filter((a) => a.kind === "coder" || a.kind === "verifier");
  const init = initial ? parseSpec(initial.schedule_json) : {};
  const [name, setName] = useState(initial?.name ?? "");
  const [agentId, setAgentId] = useState(initial?.agent_id ?? targets[0]?.id ?? "");
  const [instruction, setInstruction] = useState(initial?.instruction ?? "");
  const [kind, setKind] = useState<ScheduleKind>(initial?.schedule_kind ?? "daily");
  const [times, setTimes] = useState<string[]>(init.times?.length ? init.times : ["08:00"]);
  const [days, setDays] = useState<number[]>(init.days?.length ? init.days : [1, 2, 3, 4, 5]);
  const [every, setEvery] = useState<string>(String(init.every_minutes ?? 60));
  const [useWindow, setUseWindow] = useState<boolean>(!!init.window);
  const [from, setFrom] = useState(init.window?.from ?? "09:00");
  const [to, setTo] = useState(init.window?.to ?? "18:00");
  const [failuresFail, setFailuresFail] = useState(initial?.failures_count_as_failed ?? false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const buildSpec = (): ScheduleSpec | string => {
    if (kind === "interval") {
      const n = Number(every);
      if (!Number.isFinite(n) || n < MIN_INTERVAL) return `${MIN_INTERVAL} ${t.sched_minutes}`;
      if (useWindow && !(from < to)) return `${from} – ${to}`;
      return { every_minutes: n, window: useWindow ? { from, to } : null };
    }
    if (times.length === 0 || times.some((x) => !/^\d{2}:\d{2}$/.test(x))) return "HH:MM";
    if (kind === "weekly") {
      if (days.length === 0) return t.sched_days;
      return { times, days };
    }
    return { times };
  };

  const save = async () => {
    setError(null);
    if (!name.trim() || !instruction.trim() || !agentId) {
      setError(`${t.sched_name} / ${t.sched_instruction}`);
      return;
    }
    const spec = buildSpec();
    if (typeof spec === "string") {
      setError(spec);
      return;
    }
    setBusy(true);
    try {
      await onSave({
        name: name.trim(),
        agent_id: agentId,
        instruction: instruction.trim(),
        schedule_kind: kind,
        spec,
        failures_count_as_failed: failuresFail,
        canvas_x: initial?.canvas_x ?? null,
        canvas_y: initial?.canvas_y ?? null,
      });
      onClose();
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    } finally {
      setBusy(false);
    }
  };

  const setTime = (i: number, v: string) => setTimes((ts) => ts.map((x, j) => (j === i ? v : x)));

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-sched-dialog" role="dialog" aria-label={t.sched_title} data-testid="schedule-dialog">
        <h3 className="task-dialog-title">⏰ {t.sched_title}</h3>

        <label className="task-field">
          <span>{t.sched_name}</span>
          <input aria-label={t.sched_name} value={name} onChange={(e) => setName(e.target.value)} />
        </label>

        <label className="task-field">
          <span>{t.sched_agent}</span>
          <select aria-label={t.sched_agent} value={agentId} onChange={(e) => setAgentId(e.target.value)}>
            {targets.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
        </label>

        <label className="task-field">
          <span>{t.sched_instruction}</span>
          <textarea aria-label={t.sched_instruction} rows={4} value={instruction} onChange={(e) => setInstruction(e.target.value)} />
          <span className="task-field-hint">{t.sched_instruction_hint}</span>
        </label>

        <label className="task-field">
          <span>{t.sched_mode}</span>
          <select aria-label={t.sched_mode} value={kind} onChange={(e) => setKind(e.target.value as ScheduleKind)}>
            <option value="daily">{t.sched_mode_daily}</option>
            <option value="weekly">{t.sched_mode_weekly}</option>
            <option value="interval">{t.sched_mode_interval}</option>
          </select>
        </label>

        {kind === "weekly" && (
          <div className="task-field">
            <span>{t.sched_days}</span>
            <div className="ac-sched-days">
              {t.sched_weekdays.map((label, i) => (
                <label key={label}>
                  <input
                    type="checkbox"
                    checked={days.includes(i + 1)}
                    onChange={(e) => setDays((d) => (e.target.checked ? [...d, i + 1].sort() : d.filter((x) => x !== i + 1)))}
                  />
                  {label}
                </label>
              ))}
            </div>
          </div>
        )}

        {kind !== "interval" && (
          <div className="task-field">
            <span>{t.sched_times}</span>
            {times.map((tm, i) => (
              <div key={i} className="ac-sched-time">
                <input type="time" aria-label={`${t.sched_times} ${i + 1}`} value={tm} onChange={(e) => setTime(i, e.target.value)} />
                {times.length > 1 && (
                  <button type="button" className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => setTimes((ts) => ts.filter((_, j) => j !== i))}>
                    ✕
                  </button>
                )}
              </div>
            ))}
            {times.length < MAX_TIMES && (
              <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setTimes((ts) => [...ts, "12:00"])}>
                {t.sched_add_time}
              </button>
            )}
          </div>
        )}

        {kind === "interval" && (
          <div className="task-field">
            <span>{t.sched_every}</span>
            <div className="ac-sched-time">
              <input type="number" min={MIN_INTERVAL} aria-label={t.sched_minutes} value={every} onChange={(e) => setEvery(e.target.value)} />
              <span>{t.sched_minutes}</span>
            </div>
            <label>
              <input type="checkbox" checked={useWindow} onChange={(e) => setUseWindow(e.target.checked)} /> {t.sched_window}
            </label>
            {useWindow && (
              <div className="ac-sched-time">
                <input type="time" aria-label="from" value={from} onChange={(e) => setFrom(e.target.value)} />
                <span>–</span>
                <input type="time" aria-label="to" value={to} onChange={(e) => setTo(e.target.value)} />
              </div>
            )}
          </div>
        )}

        <label className="ac-sched-check">
          <input type="checkbox" checked={failuresFail} onChange={(e) => setFailuresFail(e.target.checked)} /> {t.sched_failures_fail}
        </label>

        {initial && (
          <div className="task-field">
            <span>{t.sched_runs}</span>
            {initial.runs.length === 0 ? (
              <span className="task-field-hint">{t.sched_no_runs}</span>
            ) : (
              <ul className="ac-sched-runs">
                {initial.runs.map((r, i) => (
                  <li key={i}>
                    {new Date(r.fired_at * 1000).toLocaleString()} —{" "}
                    {r.result === "success" ? t.sched_run_success : r.result === "failed" ? t.sched_run_failed : t.sched_run_skipped}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}

        {error && <div className="task-dialog-error">{error}</div>}
        {note && <div className="task-field-hint">{note}</div>}

        <div className="task-dialog-actions">
          {initial && onDelete && (
            <button type="button" className="aiterm-btn aiterm-btn--danger-solid" disabled={busy} onClick={() => void onDelete().then(onClose)}>
              {t.sched_delete}
            </button>
          )}
          {initial && onRunNow && (
            <button
              type="button"
              className="aiterm-btn aiterm-btn--secondary"
              disabled={busy}
              onClick={() =>
                void onRunNow()
                  .then((id) => setNote(id ? t.sched_run_now_started : t.sched_run_now_skipped))
                  .catch((e) => setError(String(e)))
              }
            >
              {t.sched_run_now}
            </button>
          )}
          <button type="button" className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.common_cancel}
          </button>
          <button type="button" className="aiterm-btn aiterm-btn--primary" disabled={busy} onClick={() => void save()}>
            {t.sched_save}
          </button>
        </div>
      </div>
    </div>
  );
}
```

`index.css` 加：

```css
.ac-sched-dialog { max-width: 460px; max-height: 90vh; overflow-y: auto; }
.ac-sched-days { display: flex; flex-wrap: wrap; gap: 8px; }
.ac-sched-time { display: flex; align-items: center; gap: 6px; margin-bottom: 4px; }
.ac-sched-check { display: flex; align-items: center; gap: 6px; margin: 8px 0; }
.ac-sched-runs { margin: 0; padding-left: 18px; font-size: 12px; }
```

驗證 `t.common_cancel` 存在（i18n 檔裡有 `common_cancel`）。

- [ ] **Step 4: 執行確認通過**：`npx vitest run src/components/AgentCanvas/ScheduleDialog.test.tsx`。突變：把最小間隔檢查拿掉，「間隔小於 5 分鐘」測試必須變紅。

- [ ] **Step 5: Commit** `git commit -m "feat(schedules): 定時器設定對話框"`

---

### Task 9: `ScheduleNode` 與畫布整合

**Files:** Create `ScheduleNode.tsx`、`ScheduleNode.test.tsx`、`scheduleStatus.ts`、`scheduleStatus.test.ts`；Modify `TopMoreMenu.tsx`、`index.tsx`、`index.css`

- [ ] **Step 1: 寫失敗測試**

`scheduleStatus.test.ts`：

```ts
import { describe, expect, it } from "vitest";

import type { ScheduleView } from "../../ipc/schedules";
import { scheduleStatus } from "./scheduleStatus";

const base = { enabled: true, disabled_reason: null, skip_streak: 0 } as ScheduleView;

describe("scheduleStatus", () => {
  it("啟用中", () => expect(scheduleStatus(base)).toBe("enabled"));
  it("手動停用", () => expect(scheduleStatus({ ...base, enabled: false })).toBe("disabled"));
  it("連續失敗停用優先於一般停用", () =>
    expect(scheduleStatus({ ...base, enabled: false, disabled_reason: "連續失敗" })).toBe("failed"));
  it("角色消失", () => expect(scheduleStatus({ ...base, enabled: false, disabled_reason: "角色已不存在" })).toBe("agent_gone"));
  it("連續跳過 3 次＝卡住（只對啟用中的）", () => {
    expect(scheduleStatus({ ...base, skip_streak: 3 })).toBe("stuck");
    expect(scheduleStatus({ ...base, skip_streak: 2 })).toBe("enabled");
  });
});
```

`ScheduleNode.test.tsx`：

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { ScheduleView } from "../../ipc/schedules";
import { ScheduleNode } from "./ScheduleNode";

const sched = (over: Partial<ScheduleView> = {}): ScheduleView => ({
  id: "s1", name: "每日測試", agent_id: "c1", instruction: "x", schedule_kind: "daily", schedule_json: '{"times":["08:00"]}',
  enabled: true, failures_count_as_failed: false, canvas_x: null, canvas_y: null, last_fired_at: 0, last_module_id: null,
  fail_streak: 0, skip_streak: 0, disabled_reason: null, runs: [], ...over,
});

function mount(s: ScheduleView, onEdit = vi.fn()) {
  render(<LocaleProvider><ScheduleNode schedule={s} x={0} y={0} onEdit={onEdit} /></LocaleProvider>);
  return onEdit;
}

describe("ScheduleNode", () => {
  it("顯示名稱與週期摘要", () => {
    mount(sched());
    expect(screen.getByText("每日測試")).toBeTruthy();
    expect(screen.getByText("每天 08:00")).toBeTruthy();
  });
  it("連續失敗停用顯示原因", () => {
    mount(sched({ enabled: false, disabled_reason: "連續失敗" }));
    expect(screen.getByText("已因連續失敗停用")).toBeTruthy();
  });
  it("上一輪卡住顯示提示", () => {
    mount(sched({ skip_streak: 3 }));
    expect(screen.getByText("上一輪卡住了")).toBeTruthy();
  });
  it("點節點開啟編輯", async () => {
    const onEdit = mount(sched());
    await userEvent.click(screen.getByTestId("schedule-node"));
    expect(onEdit).toHaveBeenCalled();
  });
  it("最近一輪結果顯示在節點上", () => {
    mount(sched({ runs: [{ schedule_id: "s1", fired_at: 1, module_id: null, result: "failed" }] }));
    expect(screen.getByText("失敗")).toBeTruthy();
  });
});
```

- [ ] **Step 2: 執行確認紅燈**。

- [ ] **Step 3: 實作**

`scheduleStatus.ts`：

```ts
import type { ScheduleView } from "../../ipc/schedules";

export type ScheduleStatus = "enabled" | "disabled" | "failed" | "agent_gone" | "stuck";

export function scheduleStatus(s: Pick<ScheduleView, "enabled" | "disabled_reason" | "skip_streak">): ScheduleStatus {
  if (!s.enabled) {
    if (s.disabled_reason === "連續失敗") return "failed";
    if (s.disabled_reason === "角色已不存在") return "agent_gone";
    return "disabled";
  }
  return s.skip_streak >= 3 ? "stuck" : "enabled";
}
```

`ScheduleNode.tsx`：

```tsx
import { useLocale } from "../../contexts/LocaleContext";
import { parseSpec, type ScheduleView } from "../../ipc/schedules";
import { summarizeSchedule } from "./scheduleSummary";
import { scheduleStatus } from "./scheduleStatus";

export const SCHEDULE_NODE_W = 200;

export function ScheduleNode({ schedule, x, y, onEdit }: { schedule: ScheduleView; x: number; y: number; onEdit: () => void }) {
  const { t } = useLocale();
  const status = scheduleStatus(schedule);
  const last = schedule.runs[0];
  const label =
    status === "failed" ? t.sched_disabled_failures
    : status === "agent_gone" ? t.sched_disabled_agent_gone
    : status === "stuck" ? t.sched_stuck
    : status === "disabled" ? t.sched_status_disabled
    : t.sched_status_enabled;
  return (
    <div
      className={`ac-sched-node ac-sched-node--${status}`}
      data-testid="schedule-node"
      data-schedule-id={schedule.id}
      style={{ left: x, top: y, width: SCHEDULE_NODE_W }}
      role="button"
      tabIndex={0}
      onClick={onEdit}
      onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && onEdit()}
    >
      <div className="ac-sched-node-title">⏰ {schedule.name}</div>
      <div className="ac-sched-node-sum">{summarizeSchedule(t, schedule.schedule_kind, parseSpec(schedule.schedule_json))}</div>
      <div className="ac-sched-node-status">{label}</div>
      {last && (
        <div className={`ac-sched-node-last ac-sched-node-last--${last.result}`}>
          {last.result === "success" ? t.sched_run_success : last.result === "failed" ? t.sched_run_failed : t.sched_run_skipped}
        </div>
      )}
    </div>
  );
}
```

CSS（`index.css`）：

```css
.ac-sched-node { position: absolute; padding: 8px 10px; border-radius: 10px; border: 1px solid var(--aiterm-border, #334155);
  background: var(--aiterm-surface-2, #1e293b); cursor: pointer; font-size: 12px; line-height: 1.5; }
.ac-sched-node--enabled { border-color: #22c55e; }
.ac-sched-node--disabled { opacity: 0.6; }
.ac-sched-node--failed, .ac-sched-node--agent_gone { border-color: #ef4444; }
.ac-sched-node--stuck { border-color: #eab308; }
.ac-sched-node-title { font-weight: 600; }
.ac-sched-node-last--failed { color: #ef4444; }
.ac-sched-node-last--success { color: #22c55e; }
.ac-sched-edges { position: absolute; left: 0; top: 0; pointer-events: none; overflow: visible; }
```

`TopMoreMenu.tsx`：新增可選 prop `onAddSchedule?: () => void`，有值時在「說明」之後加一個 `role="menuitem"` 按鈕 `t.sched_add`（模式同現有「說明」按鈕）。補一則 `TopMoreMenu` 測試（若無既有測試檔就新增 `TopMoreMenu.test.tsx`：有 prop 時顯示「新增定時器」，點下呼叫 callback；沒有 prop 時不顯示）。

`index.tsx` 整合：
1. import `ScheduleNode`、`ScheduleDialog`、`listSchedules`、`saveSchedule`、`deleteSchedule`、`runScheduleNow` 與型別。
2. 新 state：`const [schedules, setSchedules] = useState<ScheduleView[]>([]);`、`const [schedDialog, setSchedDialog] = useState<{ initial: ScheduleView | null } | null>(null);`。
3. 載入：在 `onTasksUpdated(load)` 那個 effect（約 490 行）的 `load` 內加 `listSchedules(projectId).then((s) => alive && setSchedules(s)).catch(() => {});`。
4. 位置：節點放在目標角色左側。`const schedPos = (s) => { const a = shownAgents.find((g) => g.id === s.agent_id); return { x: s.canvas_x ?? (a ? a.canvas_x - SCHEDULE_NODE_W - 40 : 40), y: s.canvas_y ?? (a ? a.canvas_y : 40) }; };`
5. 在 `<ResourceLayer …/>` 之後、`shownAgents.map` 之前渲染：先一個 `<svg className="ac-sched-edges" width={world.w} height={world.h}>`，對每個定時器畫一條從節點右緣中點到目標角色左緣中點的 `<line>`（目標不存在就不畫；座標用 `schedPos` 與 `a.canvas_x/y`，節點高度以 56、角色高度以 `NODE_H` 近似，與既有 `EdgeLayer` 以 DOM 量測不同，這裡只求連到大致位置）；再 `schedules.map((s) => <ScheduleNode key={s.id} schedule={s} {...schedPos(s)} onEdit={() => setSchedDialog({ initial: s })} />)`。
6. `<TopMoreMenu … onAddSchedule={projectId ? () => setSchedDialog({ initial: null }) : undefined} />`。
7. 對話框：`{schedDialog && projectId && <ScheduleDialog agents={agents} initial={schedDialog.initial} onSave={async (s) => { await saveSchedule(projectId, schedDialog.initial?.id ?? null, s); void refresh(); }} onDelete={schedDialog.initial ? async () => { await deleteSchedule(projectId, schedDialog.initial!.id); void refresh(); } : undefined} onRunNow={schedDialog.initial ? () => runScheduleNow(projectId, schedDialog.initial!.id) : undefined} onClose={() => setSchedDialog(null)} />}`（放在 `ReferencesDialog` 附近，約 1944 行）。
8. 啟停：節點右鍵或對話框內不做；改在對話框加「啟用」勾選——**以 `setScheduleEnabled` 呼叫**：對話框 `initial` 存在時顯示 `t.sched_enabled` 勾選並呼叫新的 prop `onToggleEnabled?: (v: boolean) => Promise<void>`（Task 8 對話框補此 prop 與測試：切換時呼叫它）。

`index.test.tsx` 既有的 mock 需補 `../../ipc/schedules`（`listSchedules` 回 `[]`），否則既有畫布測試會因未 mock 的 invoke 失敗；補完先跑 `npx vitest run src/components/AgentCanvas` 確認既有測試仍綠，再加一條「有定時器時畫布顯示節點」的整合測試。

- [ ] **Step 4: 執行確認通過**：`npx vitest run src/components/AgentCanvas`；`npx tsc -b`；`npm run lint`。

- [ ] **Step 5: Commit** `git commit -m "feat(schedules): 畫布定時器節點、選單入口與對話框整合"`

---

### Task 10: 全面驗證與實機冒煙

- [ ] **Step 1: 全套測試**

```bash
cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result|FAILED|failed"
cd .. && npx vitest run 2>&1 | tail -8
npx tsc -b && npm run lint
```
Expected: 每一行 `test result:` 皆 ok（逐行看，不只看最後一行）；vitest 全綠；tsc、lint 無錯。環境性失敗（如 aiterm-core 的 pty 測試）需註明並單獨重跑確認與本改動無關。

- [ ] **Step 2: 時區守門**：`TZ=Asia/Taipei cargo test --workspace --no-fail-fast schedules` 與 `TZ=America/Los_Angeles …` 各跑一次，確認 `due` 與 `tick` 測試不受時區影響。

- [ ] **Step 3: 冒煙（隔離 HOME＋假 claude）**：依記憶「不碰使用者那份 App 的打包版驗收法」與「假 claude」做法起 dev 版（`env -u APPLE_SIGNING_IDENTITY npm run tauri:dev`，隔離 HOME），建一個含工程師的專案，新增定時器設「每隔 5 分鐘」，驗證：到點自動建卡 → 假 claude 完成 → 模組通過後自動歸檔；再用會失敗的假 claude 驗證連續 3 次後停用。若環境無法起 GUI，退而求其次以整合式測試（`tick` ＋ `settle` 串起來）覆蓋，並在回報中明說沒有做實機畫面驗證。

- [ ] **Step 4: Commit**（若有補測試或修正）。

---

### Task 11: 文件收尾與合併

- [ ] **Step 1: 修 spec**：`docs/superpowers/specs/2026-11-16-agent-canvas-scheduler-design.md` §5 把「工程師起點的成功＝審查通過（驗收與合併是專案層級，不屬於單一輪）」寫進去，§8 三個待確認項改成「已查證」並寫結論（見本計畫開頭），§6 註明節點位置預設在目標角色左側、連線以 SVG 直線近似。
- [ ] **Step 2: CHANGELOG**：依 `feedback_changelog_before_tag`，只寫實際驗證過的行為；**不打 tag**（未經同意不推 tag）。
- [ ] **Step 3: 合併**：`git checkout master && git merge --no-ff feat/agent-canvas-scheduler`，再 `git worktree remove ../AITERM-scheduler`；**不 push**，由使用者決定。
- [ ] **Step 4: Commit** `git commit -m "docs: 定時器 spec 補查證結論"`

---

## Self-Review

**Spec coverage：** D1–D10 → Task 1–5（週期／觸發／跳過／失敗停用／歸檔／驗收開關）；§2 資料 → Task 2；§3 週期 → Task 1；§4 觸發 → Task 3、5；§5 → Task 4；§6 介面 → Task 7–9（節點、對話框、入口、編制切換停用在 Task 5 `tick`）；§7 測試 → 各任務＋Task 10。無缺口。

**型別一致：** `Fired`、`SettleEvent`、`RoundResult`、`TickCtx`、`TickReport`、`ScheduleRow`、`NewSchedule`、`Spec` 皆在 Task 1–5 定義後才被使用；前端 `ScheduleView` 與 Rust `ScheduleView`（flatten）欄位一一對應；`scheduleStatus` 的原因字串「連續失敗」「角色已不存在」與 Rust 寫入值一致。

**已知風險：** `NewAgent` 實際欄位、`SchedulerHandle` 是否 `Clone`、`commands::tasks::{project, emit_updated}` 簽章，已在對應步驟標明以 grep 確認；若與計畫不符，以實際定義為準並保持測試意圖。
