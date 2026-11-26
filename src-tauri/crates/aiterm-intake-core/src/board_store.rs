//! 共用看板（server 版）的卡片資料。
//!
//! 卡片分成兩類資料：**共用**的（標題、內容、狀態、結果…，存在這裡）與**只屬於一台電腦**的
//! （終端機分頁、對話紀錄、worktree…，留在領取者本機，不進 server）。
//! 多人同時編輯用 `version` 擋：更新要帶讀到的版本，過期就回衝突，不會靜默覆蓋別人的修改。
//! 領取沿用問題收集的做法：原子領取、15 分鐘租約、處理中續約、逾期自動放回。
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

pub const PLANNING: &str = "planning";
pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const DONE: &str = "done";

/// 領取的租約長度（秒），與問題收集相同。
pub const CLAIM_TTL_SECS: i64 = crate::store::CLAIM_TTL_SECS;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct BoardCard {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub body: String,
    pub status: String,
    pub sort_order: f64,
    pub label: Option<String>,
    pub parallel_ok: bool,
    pub interactive: bool,
    /// 結果：`success`｜`failed`（完成後才有）。
    pub outcome: Option<String>,
    pub ai_summary: Option<String>,
    pub error_message: Option<String>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub dispatched_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub archived_at: Option<i64>,
    /// 領取者（處理中才有）。
    pub claimed_by: Option<String>,
    pub claimed_at: Option<i64>,
    pub claim_expires_at: Option<i64>,
    /// 每次變動加一；更新要帶這個值，過期就是衝突。
    pub version: i64,
    /// 指派給哪個 Agent（`board_agents.id`）；一般卡片為 `None`。
    #[sqlx(default)]
    pub agent_id: Option<String>,
    /// `work`（使用者的模組／一般卡片）｜`review`｜`fix`｜`merge`｜`plan`｜`verify`。
    #[sqlx(default)]
    pub card_kind: String,
    /// review／fix 卡指向它所屬的模組（一張 work 卡）。
    #[sqlx(default)]
    pub module_id: Option<String>,
    /// review 卡：被審的那張卡。
    #[sqlx(default)]
    pub review_of: Option<String>,
    /// review 卡：`pass`｜`fail`。
    #[sqlx(default)]
    pub verdict: Option<String>,
    /// review 卡：理由（JSON 陣列字串）。
    #[sqlx(default)]
    pub review_reasons: Option<String>,
    /// 依賴的前置卡 id（JSON 陣列字串）。
    #[sqlx(default)]
    pub depends_on: Option<String>,
    /// 派工當下與完成後的 git 版本，審查用 `start..end` 取差異。
    #[sqlx(default)]
    pub start_sha: Option<String>,
    #[sqlx(default)]
    pub end_sha: Option<String>,
    /// 模組卡：使用者手動放行（審查不通過超過上限之後）。
    #[sqlx(default)]
    pub module_override: bool,
    /// 模組卡：這是一份規格（審查通過後交給 Orchestrator 拆解）。
    #[sqlx(default)]
    pub is_spec: bool,
    /// 實作卡：由哪一份規格拆出來的。
    #[sqlx(default)]
    pub from_spec: Option<String>,
}

/// Agent 協作的 Agent（Coder／Reviewer／Orchestrator）。只放「共用」欄位；
/// 技能（資料庫、自訂、系統連線）是各台電腦的環境，不放進 server。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct BoardAgent {
    pub id: String,
    pub project_id: String,
    /// `coder`｜`reviewer`｜`orchestrator`。
    pub kind: String,
    pub name: String,
    pub role: String,
    pub prompt_prefix: String,
    pub canvas_x: f64,
    pub canvas_y: f64,
    /// Coder：產出規格。
    pub produces_spec: bool,
    /// 派工時給 claude 的 `--model`；`None`＝沿用預設。
    pub model: Option<String>,
    /// 用哪個 AI 工具：`claude`｜`codex`。
    pub cli: String,
    /// 非 `None`＝暫停中，指向造成暫停的那張卡。
    pub paused_card_id: Option<String>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct BoardNote {
    pub id: i64,
    pub project_id: String,
    /// 署名；使用者自己加的留空。
    pub author: String,
    pub text: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct BoardAttachment {
    pub id: String,
    pub card_id: String,
    pub filename: String,
    pub path: String,
    pub mime: String,
    pub size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct BoardComment {
    pub id: i64,
    pub card_id: String,
    pub author: String,
    pub text: String,
    pub created_at: i64,
}

/// 列表用：卡片加上附件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoardCardView {
    #[serde(flatten)]
    pub card: BoardCard,
    pub attachments: Vec<BoardAttachment>,
    pub comment_count: i64,
}

pub struct NewCard<'a> {
    pub project_id: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub parallel_ok: bool,
    pub interactive: bool,
    pub label: Option<&'a str>,
    pub created_by: &'a str,
}

/// 更新或移動的結果。
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// 成功，附上新的卡片。
    Ok(Box<BoardCard>),
    /// 版本過期：別人先改過了。
    Conflict,
    NotFound,
    /// 不允許（例如處理中的卡片不能改標題、不合法的欄位移動）。
    Refused(String),
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS board_cards (
            id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'planning', sort_order REAL NOT NULL DEFAULT 0, label TEXT,
            parallel_ok INTEGER NOT NULL DEFAULT 1, interactive INTEGER NOT NULL DEFAULT 0,
            outcome TEXT, ai_summary TEXT, error_message TEXT, created_by TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, dispatched_at INTEGER, finished_at INTEGER, archived_at INTEGER,
            claimed_by TEXT, claimed_at INTEGER, claim_expires_at INTEGER, version INTEGER NOT NULL DEFAULT 1)",
        "CREATE INDEX IF NOT EXISTS board_cards_project ON board_cards(project_id, status)",
        "CREATE TABLE IF NOT EXISTS board_attachments (
            id TEXT PRIMARY KEY NOT NULL, card_id TEXT NOT NULL, filename TEXT NOT NULL, path TEXT NOT NULL, mime TEXT NOT NULL, size INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS board_comments (
            id INTEGER PRIMARY KEY AUTOINCREMENT, card_id TEXT NOT NULL, author TEXT NOT NULL, text TEXT NOT NULL, created_at INTEGER NOT NULL)",
    ] {
        sqlx::query(sql).execute(pool).await?;
    }
    // 欄位升級：舊資料庫沒有 Agent 協作的欄位。欄位已存在時 ALTER 會失敗，那是正常的，所以刻意丟掉錯誤。
    for col in [
        "agent_id TEXT",
        "card_kind TEXT NOT NULL DEFAULT 'work'",
        "module_id TEXT",
        "review_of TEXT",
        "verdict TEXT",
        "review_reasons TEXT",
        "depends_on TEXT",
        "start_sha TEXT",
        "end_sha TEXT",
        "module_override INTEGER NOT NULL DEFAULT 0",
        "is_spec INTEGER NOT NULL DEFAULT 0",
        "from_spec TEXT",
    ] {
        let _ = sqlx::query(&format!("ALTER TABLE board_cards ADD COLUMN {col}")).execute(pool).await;
    }
    for sql in [
        "CREATE TABLE IF NOT EXISTS board_agents (
            id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL, kind TEXT NOT NULL, name TEXT NOT NULL, role TEXT NOT NULL DEFAULT '',
            prompt_prefix TEXT NOT NULL DEFAULT '', canvas_x REAL NOT NULL DEFAULT 0, canvas_y REAL NOT NULL DEFAULT 0,
            produces_spec INTEGER NOT NULL DEFAULT 0, model TEXT, cli TEXT NOT NULL DEFAULT 'claude', paused_card_id TEXT,
            created_by TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, version INTEGER NOT NULL DEFAULT 1)",
        "CREATE INDEX IF NOT EXISTS board_agents_project ON board_agents(project_id)",
        "CREATE TABLE IF NOT EXISTS board_notes (
            id INTEGER PRIMARY KEY AUTOINCREMENT, project_id TEXT NOT NULL, author TEXT NOT NULL DEFAULT '', text TEXT NOT NULL, created_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS board_schedules (
            id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL, name TEXT NOT NULL, agent_id TEXT NOT NULL, instruction TEXT NOT NULL,
            schedule_kind TEXT NOT NULL, schedule_json TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, failures_count_as_failed INTEGER NOT NULL DEFAULT 0,
            canvas_x REAL, canvas_y REAL, last_fired_at INTEGER NOT NULL DEFAULT 0, last_card_id TEXT, fail_streak INTEGER NOT NULL DEFAULT 0,
            skip_streak INTEGER NOT NULL DEFAULT 0, disabled_reason TEXT, created_by TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        "CREATE INDEX IF NOT EXISTS board_schedules_project ON board_schedules(project_id)",
        "CREATE TABLE IF NOT EXISTS board_card_wait (card_id TEXT PRIMARY KEY NOT NULL, until INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 1, reason TEXT NOT NULL DEFAULT '')",
        // 卡片處理到一半被打斷（server 重啟、額度用完）後要接著做：記 Claude Code 的對話 id 與起始版本。
        "CREATE TABLE IF NOT EXISTS board_card_session (card_id TEXT PRIMARY KEY NOT NULL, session_id TEXT NOT NULL, start_sha TEXT)",
        "CREATE TABLE IF NOT EXISTS board_card_verification (card_id TEXT PRIMARY KEY NOT NULL, report TEXT NOT NULL, passed INTEGER NOT NULL, created_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS board_schedule_runs (schedule_id TEXT NOT NULL, fired_at INTEGER NOT NULL, card_id TEXT, result TEXT NOT NULL)",
        "CREATE INDEX IF NOT EXISTS board_schedule_runs_idx ON board_schedule_runs(schedule_id, fired_at)",
    ] {
        sqlx::query(sql).execute(pool).await?;
    }
    Ok(())
}

async fn next_order(pool: &SqlitePool, project_id: &str, status: &str) -> Result<f64, sqlx::Error> {
    let max: Option<f64> = sqlx::query_scalar("SELECT CAST(MAX(sort_order) AS REAL) FROM board_cards WHERE project_id = ? AND status = ?").bind(project_id).bind(status).fetch_one(pool).await?;
    Ok(max.unwrap_or(0.0) + 1.0)
}

pub async fn create_card(pool: &SqlitePool, c: NewCard<'_>) -> Result<BoardCard, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let t = now();
    let order = next_order(pool, c.project_id, PLANNING).await?;
    sqlx::query(
        "INSERT INTO board_cards (id, project_id, title, body, status, sort_order, label, parallel_ok, interactive, created_by, created_at, updated_at, version)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,1)",
    )
    .bind(&id).bind(c.project_id).bind(c.title.trim()).bind(c.body).bind(PLANNING).bind(order).bind(c.label.map(str::trim).filter(|l| !l.is_empty())).bind(c.parallel_ok).bind(c.interactive).bind(c.created_by).bind(t).bind(t)
    .execute(pool).await?;
    Ok(get_card(pool, &id).await?.expect("剛建立的卡片"))
}

pub async fn get_card(pool: &SqlitePool, id: &str) -> Result<Option<BoardCard>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_cards WHERE id = ?").bind(id).fetch_optional(pool).await
}

/// 專案的卡片（依欄位與排序）。`archived` 為真只列封存的，否則只列看板上的。
pub async fn list_cards(pool: &SqlitePool, project_id: &str, archived: bool) -> Result<Vec<BoardCardView>, sqlx::Error> {
    let sql = if archived {
        "SELECT * FROM board_cards WHERE project_id = ? AND archived_at IS NOT NULL ORDER BY archived_at DESC"
    } else {
        "SELECT * FROM board_cards WHERE project_id = ? AND archived_at IS NULL ORDER BY sort_order, created_at"
    };
    let cards: Vec<BoardCard> = sqlx::query_as(sql).bind(project_id).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(cards.len());
    for card in cards {
        let attachments = list_attachments(pool, &card.id).await?;
        let comment_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_comments WHERE card_id = ?").bind(&card.id).fetch_one(pool).await?;
        out.push(BoardCardView { card, attachments, comment_count });
    }
    Ok(out)
}

pub struct Edit<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub parallel_ok: bool,
    pub interactive: bool,
    pub label: Option<&'a str>,
}

/// 編輯卡片內容。`version` 是呼叫端讀到的版本；處理中的卡片不能改（會跟正在跑的 Agent 打架）。
pub async fn update_card(pool: &SqlitePool, id: &str, version: i64, e: Edit<'_>) -> Result<Change, sqlx::Error> {
    let Some(cur) = get_card(pool, id).await? else { return Ok(Change::NotFound) };
    if cur.status == RUNNING {
        return Ok(Change::Refused("處理中的卡片不能編輯".into()));
    }
    if e.title.trim().is_empty() {
        return Ok(Change::Refused("標題不能是空的".into()));
    }
    let r = sqlx::query("UPDATE board_cards SET title = ?, body = ?, parallel_ok = ?, interactive = ?, label = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?")
        .bind(e.title.trim()).bind(e.body).bind(e.parallel_ok).bind(e.interactive).bind(e.label.map(str::trim).filter(|l| !l.is_empty())).bind(now()).bind(id).bind(version)
        .execute(pool).await?;
    if r.rows_affected() == 0 {
        return Ok(Change::Conflict);
    }
    Ok(Change::Ok(Box::new(get_card(pool, id).await?.expect("剛更新的卡片"))))
}

fn transition_ok(from: &str, to: &str) -> bool {
    matches!((from, to), (PLANNING, QUEUED) | (QUEUED, PLANNING) | (PLANNING, PLANNING) | (QUEUED, QUEUED))
}

/// 把排序值放在 `after_id` 之後、`before_id` 之前（任一為空＝欄位最上或最下）。
async fn order_between(pool: &SqlitePool, project_id: &str, status: &str, after_id: Option<&str>, before_id: Option<&str>, self_id: &str) -> Result<f64, sqlx::Error> {
    async fn order_of(pool: &SqlitePool, id: &str) -> Result<Option<f64>, sqlx::Error> {
        sqlx::query_scalar("SELECT CAST(sort_order AS REAL) FROM board_cards WHERE id = ?").bind(id).fetch_optional(pool).await
    }
    let lo = match after_id {
        Some(id) => order_of(pool, id).await?,
        None => None,
    };
    let hi = match before_id {
        Some(id) => order_of(pool, id).await?,
        None => None,
    };
    let (min, max): (Option<f64>, Option<f64>) = sqlx::query_as("SELECT CAST(MIN(sort_order) AS REAL), CAST(MAX(sort_order) AS REAL) FROM board_cards WHERE project_id = ? AND status = ? AND archived_at IS NULL AND id != ?")
        .bind(project_id).bind(status).bind(self_id).fetch_one(pool).await?;
    let lo = lo.unwrap_or_else(|| min.unwrap_or(1.0) - 1.0);
    let hi = hi.unwrap_or_else(|| max.unwrap_or(0.0) + 2.0);
    Ok((lo + hi) / 2.0)
}

/// 移動卡片：手動只能在計畫中與待派送之間移（進入執行中與完成走領取與交結果）。
/// 排序用「放在誰之後、誰之前」表達，多人同時拖曳時不會互相踩到。
pub async fn move_card(pool: &SqlitePool, id: &str, version: i64, to_status: &str, after_id: Option<&str>, before_id: Option<&str>) -> Result<Change, sqlx::Error> {
    move_card_at(pool, id, version, to_status, after_id, before_id, None).await
}

/// 同 `move_card`；`sort_order` 有值時直接用它（AITerm 依畫面上看到的鄰居算好的值），否則依鄰居算。
pub async fn move_card_at(pool: &SqlitePool, id: &str, version: i64, to_status: &str, after_id: Option<&str>, before_id: Option<&str>, sort_order: Option<f64>) -> Result<Change, sqlx::Error> {
    let Some(cur) = get_card(pool, id).await? else { return Ok(Change::NotFound) };
    if !transition_ok(&cur.status, to_status) {
        return Ok(Change::Refused(format!("不能從 {} 移到 {}", cur.status, to_status)));
    }
    let order = match sort_order {
        Some(o) if o.is_finite() => o,
        _ => order_between(pool, &cur.project_id, to_status, after_id, before_id, id).await?,
    };
    let r = sqlx::query("UPDATE board_cards SET status = ?, sort_order = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?")
        .bind(to_status).bind(order).bind(now()).bind(id).bind(version).execute(pool).await?;
    if r.rows_affected() == 0 {
        return Ok(Change::Conflict);
    }
    Ok(Change::Ok(Box::new(get_card(pool, id).await?.expect("剛移動的卡片"))))
}

/// 封存只限已完成的卡片（把進行中的藏起來等於讓工作憑空消失）。
pub async fn set_archived(pool: &SqlitePool, id: &str, archived: bool) -> Result<bool, sqlx::Error> {
    let r = if archived {
        sqlx::query("UPDATE board_cards SET archived_at = ?, updated_at = ?, version = version + 1 WHERE id = ? AND status = ? AND archived_at IS NULL").bind(now()).bind(now()).bind(id).bind(DONE).execute(pool).await?
    } else {
        sqlx::query("UPDATE board_cards SET archived_at = NULL, updated_at = ?, version = version + 1 WHERE id = ? AND archived_at IS NOT NULL").bind(now()).bind(id).execute(pool).await?
    };
    Ok(r.rows_affected() == 1)
}

/// 把專案裡所有已完成、還沒封存的卡片封存，回傳張數。
pub async fn archive_all_done(pool: &SqlitePool, project_id: &str) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("UPDATE board_cards SET archived_at = ?, updated_at = ?, version = version + 1 WHERE project_id = ? AND status = ? AND archived_at IS NULL").bind(now()).bind(now()).bind(project_id).bind(DONE).execute(pool).await?;
    Ok(r.rows_affected())
}

/// 刪除卡片與它的附件紀錄、留言（處理中的不能刪）。回傳附件檔案路徑讓呼叫端清掉檔案；`None`＝沒刪成。
pub async fn delete_card(pool: &SqlitePool, id: &str) -> Result<Option<Vec<String>>, sqlx::Error> {
    let r = sqlx::query("DELETE FROM board_cards WHERE id = ? AND status != ?").bind(id).bind(RUNNING).execute(pool).await?;
    if r.rows_affected() == 0 {
        return Ok(None);
    }
    let paths: Vec<String> = sqlx::query_scalar("SELECT path FROM board_attachments WHERE card_id = ?").bind(id).fetch_all(pool).await?;
    sqlx::query("DELETE FROM board_attachments WHERE card_id = ?").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM board_comments WHERE card_id = ?").bind(id).execute(pool).await?;
    Ok(Some(paths))
}

// ---- 編排（server 執行）用 ----

/// 編排產生的卡片（審查、修正、合併）。
pub struct NewWorkflowCard<'a> {
    pub project_id: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    /// `review`｜`fix`｜`merge`。
    pub kind: &'a str,
    pub agent_id: Option<&'a str>,
    pub module_id: Option<&'a str>,
    pub review_of: Option<&'a str>,
    /// 審查與修正直接排進待派送；合併卡放規劃中等管理員核准。
    pub status: &'a str,
    /// 實作卡（`work`）：依賴的前置卡 id（JSON 陣列字串）與來源規格。
    pub depends_on: Option<&'a str>,
    pub from_spec: Option<&'a str>,
}

pub async fn create_workflow_card(pool: &SqlitePool, c: NewWorkflowCard<'_>) -> Result<BoardCard, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let t = now();
    let order = next_order(pool, c.project_id, c.status).await?;
    sqlx::query(
        "INSERT INTO board_cards (id, project_id, title, body, status, sort_order, created_by, created_at, updated_at, version, agent_id, card_kind, module_id, review_of, depends_on, from_spec)
         VALUES (?,?,?,?,?,?,?,?,?,1,?,?,?,?,?,?)",
    )
    .bind(&id).bind(c.project_id).bind(c.title).bind(c.body).bind(c.status).bind(order).bind("server").bind(t).bind(t).bind(c.agent_id).bind(c.kind).bind(c.module_id).bind(c.review_of).bind(c.depends_on).bind(c.from_spec)
    .execute(pool).await?;
    Ok(get_card(pool, &id).await?.expect("剛建立的卡片"))
}

/// 專案裡所有沒封存的卡片（沒有附件資訊，編排用）。
pub async fn project_cards(pool: &SqlitePool, project_id: &str) -> Result<Vec<BoardCard>, sqlx::Error> {
    sqlx::query_as::<_, BoardCard>("SELECT * FROM board_cards WHERE project_id = ? AND archived_at IS NULL ORDER BY sort_order, created_at").bind(project_id).fetch_all(pool).await
}

/// 這個 Agent 現在有沒有處理中的卡片（一個 Agent 一次只做一張）。
pub async fn agent_busy(pool: &SqlitePool, agent_id: &str) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM board_cards WHERE agent_id = ? AND status = ?").bind(agent_id).bind(RUNNING).fetch_one(pool).await? > 0)
}

/// 記下這張卡做之前與做完之後的 git 版本（審查用 `start..end` 取差異）。
pub async fn set_shas(pool: &SqlitePool, id: &str, start: Option<&str>, end: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_cards SET start_sha = ?, end_sha = ?, updated_at = ?, version = version + 1 WHERE id = ?").bind(start).bind(end).bind(now()).bind(id).execute(pool).await?;
    Ok(())
}

/// 審查卡寫入結論（`pass`｜`fail`）與理由。
pub async fn set_verdict(pool: &SqlitePool, id: &str, verdict: &str, reasons: &[String]) -> Result<(), sqlx::Error> {
    let json = serde_json::to_string(reasons).unwrap_or_else(|_| "[]".into());
    sqlx::query("UPDATE board_cards SET verdict = ?, review_reasons = ?, updated_at = ?, version = version + 1 WHERE id = ?").bind(verdict).bind(json).bind(now()).bind(id).execute(pool).await?;
    Ok(())
}

/// 不經領取直接結案（合併卡經管理員核准後用）。
pub async fn complete_card(pool: &SqlitePool, id: &str, outcome: &str, summary: &str) -> Result<bool, sqlx::Error> {
    let t = now();
    let r = sqlx::query("UPDATE board_cards SET status = ?, outcome = ?, ai_summary = ?, finished_at = ?, claim_expires_at = NULL, updated_at = ?, version = version + 1 WHERE id = ? AND status != ?")
        .bind(DONE).bind(outcome).bind(summary).bind(t).bind(t).bind(id).bind(DONE).execute(pool).await?;
    Ok(r.rows_affected() == 1)
}

// ---- 定時器（server 觸發）----

/// 連續失敗幾輪就自動停用。
pub const SCHEDULE_MAX_FAIL_STREAK: i64 = 3;
/// 每個定時器保留最近幾輪的紀錄。
pub const SCHEDULE_RUNS_KEPT: i64 = 5;
/// `disabled_reason` 的值；前端 `scheduleStatus.ts` 要跟這些字串一致。
pub const REASON_FAILURES: &str = "連續失敗";
pub const REASON_AGENT_GONE: &str = "角色已不存在";

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct BoardSchedule {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub agent_id: String,
    pub instruction: String,
    pub schedule_kind: String,
    pub schedule_json: String,
    pub enabled: bool,
    pub failures_count_as_failed: bool,
    pub canvas_x: Option<f64>,
    pub canvas_y: Option<f64>,
    /// 上次觸發（或建立、啟用）的 unix 秒；到點判斷以它為基準。
    pub last_fired_at: i64,
    /// 進行中那一輪的卡片；結算後清掉。
    pub last_card_id: Option<String>,
    pub fail_streak: i64,
    pub skip_streak: i64,
    pub disabled_reason: Option<String>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl BoardSchedule {
    pub fn spec(&self) -> Result<crate::schedule::Spec, String> {
        serde_json::from_str(&self.schedule_json).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct BoardScheduleRun {
    pub schedule_id: String,
    pub fired_at: i64,
    pub card_id: Option<String>,
    /// `success`｜`failed`｜`skipped`
    pub result: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleInput {
    pub name: String,
    pub agent_id: String,
    pub instruction: String,
    pub schedule_kind: String,
    pub spec: crate::schedule::Spec,
    #[serde(default)]
    pub failures_count_as_failed: bool,
    #[serde(default)]
    pub canvas_x: Option<f64>,
    #[serde(default)]
    pub canvas_y: Option<f64>,
}

/// 檢查輸入：週期規則合法、名稱與指令不空、目標是同專案的工程師（server 只能叫起工程師）。
async fn check_schedule(pool: &SqlitePool, project_id: &str, i: &ScheduleInput) -> Result<(String, String), String> {
    crate::schedule::validate(&i.schedule_kind, &i.spec)?;
    let (name, instruction) = (i.name.trim().to_string(), i.instruction.trim().to_string());
    if name.is_empty() {
        return Err("名稱不能是空的".into());
    }
    if instruction.is_empty() {
        return Err("指令不能是空的".into());
    }
    match get_agent(pool, &i.agent_id).await.map_err(|e| e.to_string())? {
        None => Err("找不到這個角色".into()),
        Some(a) if a.project_id != project_id => Err("角色與定時器不在同一個專案".into()),
        Some(a) if a.kind != "coder" => Err("定時器只能叫起工程師".into()),
        Some(_) => Ok((name, instruction)),
    }
}

/// 建立。`now` 當作 `last_fired_at` 的基準（避免一建立就因為「今天的時間點已經過了」而立刻觸發）。
pub async fn create_schedule(pool: &SqlitePool, project_id: &str, i: &ScheduleInput, by: &str, now: i64) -> Result<BoardSchedule, String> {
    let (name, instruction) = check_schedule(pool, project_id, i).await?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO board_schedules (id, project_id, name, agent_id, instruction, schedule_kind, schedule_json, failures_count_as_failed, canvas_x, canvas_y, last_fired_at, created_by, created_at, updated_at)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id).bind(project_id).bind(name).bind(&i.agent_id).bind(instruction).bind(&i.schedule_kind).bind(serde_json::to_string(&i.spec).map_err(|e| e.to_string())?)
    .bind(i.failures_count_as_failed).bind(i.canvas_x).bind(i.canvas_y).bind(now).bind(by).bind(now).bind(now)
    .execute(pool).await.map_err(|e| e.to_string())?;
    get_schedule(pool, &id).await.map_err(|e| e.to_string())?.ok_or_else(|| "剛建立的定時器不見了".into())
}

/// 更新設定；不動執行狀態（`last_*`、streak）。
pub async fn update_schedule(pool: &SqlitePool, id: &str, i: &ScheduleInput) -> Result<BoardSchedule, String> {
    let cur = get_schedule(pool, id).await.map_err(|e| e.to_string())?.ok_or_else(|| "找不到這個定時器".to_string())?;
    let (name, instruction) = check_schedule(pool, &cur.project_id, i).await?;
    sqlx::query("UPDATE board_schedules SET name = ?, agent_id = ?, instruction = ?, schedule_kind = ?, schedule_json = ?, failures_count_as_failed = ?, canvas_x = ?, canvas_y = ?, updated_at = ? WHERE id = ?")
        .bind(name).bind(&i.agent_id).bind(instruction).bind(&i.schedule_kind).bind(serde_json::to_string(&i.spec).map_err(|e| e.to_string())?)
        .bind(i.failures_count_as_failed).bind(i.canvas_x).bind(i.canvas_y).bind(now()).bind(id)
        .execute(pool).await.map_err(|e| e.to_string())?;
    get_schedule(pool, id).await.map_err(|e| e.to_string())?.ok_or_else(|| "找不到這個定時器".into())
}

pub async fn list_schedules(pool: &SqlitePool, project_id: &str) -> Result<Vec<BoardSchedule>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_schedules WHERE project_id = ? ORDER BY created_at, id").bind(project_id).fetch_all(pool).await
}

/// 所有專案的定時器（掃描用）。
pub async fn all_schedules(pool: &SqlitePool) -> Result<Vec<BoardSchedule>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_schedules ORDER BY created_at, id").fetch_all(pool).await
}

pub async fn get_schedule(pool: &SqlitePool, id: &str) -> Result<Option<BoardSchedule>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_schedules WHERE id = ?").bind(id).fetch_optional(pool).await
}

pub async fn delete_schedule(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    sqlx::query("DELETE FROM board_schedule_runs WHERE schedule_id = ?").bind(id).execute(pool).await?;
    Ok(sqlx::query("DELETE FROM board_schedules WHERE id = ?").bind(id).execute(pool).await?.rows_affected() == 1)
}

/// 啟用時清掉連續失敗／跳過與停用原因，並把基準時間設成現在（停用期間錯過的時間點不補，也避免一啟用就立刻觸發）。
pub async fn set_schedule_enabled(pool: &SqlitePool, id: &str, enabled: bool, now: i64) -> Result<bool, sqlx::Error> {
    let r = if enabled {
        sqlx::query("UPDATE board_schedules SET enabled = 1, fail_streak = 0, skip_streak = 0, disabled_reason = NULL, last_fired_at = ?, updated_at = ? WHERE id = ?").bind(now).bind(now).bind(id).execute(pool).await?
    } else {
        sqlx::query("UPDATE board_schedules SET enabled = 0, disabled_reason = NULL, updated_at = ? WHERE id = ?").bind(now).bind(id).execute(pool).await?
    };
    Ok(r.rows_affected() == 1)
}

/// 停用並記下原因（`set_schedule_enabled(.., true, ..)` 會清掉）。
pub async fn disable_schedule(pool: &SqlitePool, id: &str, reason: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_schedules SET enabled = 0, disabled_reason = ? WHERE id = ?").bind(reason).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn record_schedule_run(pool: &SqlitePool, id: &str, fired_at: i64, card_id: Option<&str>, result: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO board_schedule_runs (schedule_id, fired_at, card_id, result) VALUES (?,?,?,?)").bind(id).bind(fired_at).bind(card_id).bind(result).execute(pool).await?;
    sqlx::query("DELETE FROM board_schedule_runs WHERE schedule_id = ?1 AND rowid NOT IN (SELECT rowid FROM board_schedule_runs WHERE schedule_id = ?1 ORDER BY fired_at DESC, rowid DESC LIMIT ?2)")
        .bind(id).bind(SCHEDULE_RUNS_KEPT).execute(pool).await?;
    Ok(())
}

pub async fn list_schedule_runs(pool: &SqlitePool, id: &str) -> Result<Vec<BoardScheduleRun>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_schedule_runs WHERE schedule_id = ? ORDER BY fired_at DESC, rowid DESC").bind(id).fetch_all(pool).await
}

/// 觸發成功：記下這一輪的卡、推進基準時間、清掉跳過計數。
pub async fn mark_schedule_fired(pool: &SqlitePool, id: &str, fired_at: i64, card_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_schedules SET last_fired_at = ?, last_card_id = ?, skip_streak = 0 WHERE id = ?").bind(fired_at).bind(card_id).bind(id).execute(pool).await?;
    Ok(())
}

/// 上一輪還沒結案而跳過：記一筆 skipped、跳過計數加一，照樣推進基準時間（不然每個 tick 都會再試）。
pub async fn mark_schedule_skipped(pool: &SqlitePool, id: &str, at: i64, card_id: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_schedules SET skip_streak = skip_streak + 1, last_fired_at = ? WHERE id = ?").bind(at).bind(id).execute(pool).await?;
    record_schedule_run(pool, id, at, card_id, "skipped").await
}

/// 一輪結案：清掉進行中的卡；成功歸零連續失敗，失敗加一。回傳新的連續失敗次數。
pub async fn settle_schedule_round(pool: &SqlitePool, id: &str, success: bool) -> Result<i64, sqlx::Error> {
    if success {
        sqlx::query("UPDATE board_schedules SET fail_streak = 0, last_card_id = NULL WHERE id = ?").bind(id).execute(pool).await?;
        Ok(0)
    } else {
        sqlx::query("UPDATE board_schedules SET fail_streak = fail_streak + 1, last_card_id = NULL WHERE id = ?").bind(id).execute(pool).await?;
        Ok(sqlx::query_scalar::<_, i64>("SELECT fail_streak FROM board_schedules WHERE id = ?").bind(id).fetch_one(pool).await?)
    }
}

/// 沒有進行中的輪了（那張卡被刪、或人工清掉）。
pub async fn clear_schedule_round(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_schedules SET last_card_id = NULL WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

/// 成功的一輪把已完成的卡（模組本身加整條鏈）歸檔：看板上不堆積定時產生的卡；進行中的不碰。
pub async fn archive_round(pool: &SqlitePool, module_id: &str, at: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE board_cards SET archived_at = ?1 WHERE (id = ?2 OR module_id = ?2) AND status = 'done' AND archived_at IS NULL").bind(at).bind(module_id).execute(pool).await?.rows_affected())
}

// ---- server 對 Coder 產出的驗證 ----

/// 記下 server 在 Coder 完成後實際跑的驗證（變更了哪些檔案、專案設定的測試指令有沒有過）。審查會看到它。
pub async fn set_verification(pool: &SqlitePool, card_id: &str, report: &str, passed: bool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR REPLACE INTO board_card_verification (card_id, report, passed, created_at) VALUES (?,?,?,?)").bind(card_id).bind(report).bind(passed).bind(now()).execute(pool).await?;
    Ok(())
}

pub async fn get_verification(pool: &SqlitePool, card_id: &str) -> Result<Option<(String, bool)>, sqlx::Error> {
    sqlx::query_as::<_, (String, bool)>("SELECT report, passed FROM board_card_verification WHERE card_id = ?").bind(card_id).fetch_optional(pool).await
}

// ---- 額度用完等待續跑 ----

/// 額度／速率限制最多自動等幾次；超過就當失敗，交給人處理。
pub const QUOTA_MAX_ATTEMPTS: i64 = 12;

/// 這張卡要等到 `until`（unix 秒）才能再派；累計等了幾次。回傳目前的累計次數。
pub async fn set_card_wait(pool: &SqlitePool, card_id: &str, until: i64, reason: &str) -> Result<i64, sqlx::Error> {
    sqlx::query("INSERT INTO board_card_wait (card_id, until, attempts, reason) VALUES (?,?,1,?) ON CONFLICT(card_id) DO UPDATE SET until = excluded.until, attempts = attempts + 1, reason = excluded.reason")
        .bind(card_id).bind(until).bind(reason).execute(pool).await?;
    sqlx::query_scalar::<_, i64>("SELECT attempts FROM board_card_wait WHERE card_id = ?").bind(card_id).fetch_one(pool).await
}

/// 這張卡現在是不是還在等（`until` 還沒到）。
pub async fn card_waiting(pool: &SqlitePool, card_id: &str, now: i64) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM board_card_wait WHERE card_id = ? AND until > ?").bind(card_id).bind(now).fetch_one(pool).await? > 0)
}

pub async fn card_wait_attempts(pool: &SqlitePool, card_id: &str) -> Result<i64, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT attempts FROM board_card_wait WHERE card_id = ?").bind(card_id).fetch_optional(pool).await?.unwrap_or(0))
}

pub async fn clear_card_wait(pool: &SqlitePool, card_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM board_card_wait WHERE card_id = ?").bind(card_id).execute(pool).await?;
    Ok(())
}

// ---- 中途接續：處理到一半的對話 ----

/// 一張卡上次處理留下的 Claude Code 對話（接續用）。
#[derive(Debug, Clone, PartialEq)]
pub struct CardSession {
    pub session_id: String,
    /// 第一次開工時 worktree 的 HEAD；接續時沿用，審查的 diff 才不會漏掉上次已 commit 的部分。
    pub start_sha: Option<String>,
}

pub async fn get_session(pool: &SqlitePool, card_id: &str) -> Result<Option<CardSession>, sqlx::Error> {
    let row: Option<(String, Option<String>)> = sqlx::query_as("SELECT session_id, start_sha FROM board_card_session WHERE card_id = ?").bind(card_id).fetch_optional(pool).await?;
    Ok(row.map(|(session_id, start_sha)| CardSession { session_id, start_sha }))
}

/// 記下（或換掉）這張卡現在用的對話。
pub async fn set_session(pool: &SqlitePool, card_id: &str, session_id: &str, start_sha: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO board_card_session (card_id, session_id, start_sha) VALUES (?,?,?) ON CONFLICT(card_id) DO UPDATE SET session_id = excluded.session_id, start_sha = excluded.start_sha")
        .bind(card_id).bind(session_id).bind(start_sha).execute(pool).await?;
    Ok(())
}

/// 所有還記著的對話 id（server 啟動時用來找出上一個行程留下的孤兒 Claude）。
pub async fn all_session_ids(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT session_id FROM board_card_session").fetch_all(pool).await
}

/// 卡片做完（成功或失敗）就不再接續；回傳被清掉的對話 id，呼叫端順便刪掉磁碟上的對話檔。
pub async fn clear_session(pool: &SqlitePool, card_id: &str) -> Result<Option<String>, sqlx::Error> {
    let prev = get_session(pool, card_id).await?.map(|s| s.session_id);
    sqlx::query("DELETE FROM board_card_session WHERE card_id = ?").bind(card_id).execute(pool).await?;
    Ok(prev)
}

/// 放回待派送並留下原因（給人看：為什麼這張卡又排隊了）。只有處理中的才放。
pub async fn requeue_with_note(pool: &SqlitePool, id: &str, who: &str, note: &str, now: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE board_cards SET status = ?, claimed_by = NULL, claimed_at = NULL, claim_expires_at = NULL, error_message = ?, updated_at = ?, version = version + 1 WHERE id = ? AND status = ? AND claimed_by = ?")
        .bind(QUEUED).bind(note).bind(now).bind(id).bind(RUNNING).bind(who).execute(pool).await?;
    Ok(r.rows_affected() == 1)
}

// ---- 領取 ----

/// 原子領取：只有 `queued`（沒封存）領得到，同一張只有一個人成功。
pub async fn claim_card(pool: &SqlitePool, id: &str, who: &str, now: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query(
        "UPDATE board_cards SET status = ?, claimed_by = ?, claimed_at = ?, claim_expires_at = ?, dispatched_at = ?, error_message = NULL, outcome = NULL, updated_at = ?, version = version + 1
         WHERE id = ? AND status = ? AND archived_at IS NULL",
    )
    .bind(RUNNING).bind(who).bind(now).bind(now + CLAIM_TTL_SECS).bind(now).bind(now).bind(id).bind(QUEUED)
    .execute(pool).await?;
    Ok(r.rows_affected() == 1)
}

/// 續約：只有領取者、而且卡片還在處理中。
pub async fn renew_claim(pool: &SqlitePool, id: &str, who: &str, now: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE board_cards SET claim_expires_at = ? WHERE id = ? AND status = ? AND claimed_by = ?").bind(now + CLAIM_TTL_SECS).bind(id).bind(RUNNING).bind(who).execute(pool).await?;
    Ok(r.rows_affected() == 1)
}

/// 放回待派送。`only_by` 有值時必須是該成員領的（管理員傳 `None`）。
pub async fn release_claim(pool: &SqlitePool, id: &str, only_by: Option<&str>, now: i64) -> Result<bool, sqlx::Error> {
    let mut sql = String::from("UPDATE board_cards SET status = ?, claimed_by = NULL, claimed_at = NULL, claim_expires_at = NULL, updated_at = ?, version = version + 1 WHERE id = ? AND status = ?");
    if only_by.is_some() {
        sql.push_str(" AND claimed_by = ?");
    }
    let mut q = sqlx::query(&sql).bind(QUEUED).bind(now).bind(id).bind(RUNNING);
    if let Some(w) = only_by {
        q = q.bind(w);
    }
    Ok(q.execute(pool).await?.rows_affected() == 1)
}

/// 領取者交結果：完成（`success`／`failed`），記摘要與錯誤。
pub async fn finish_claim(pool: &SqlitePool, id: &str, who: &str, outcome: &str, summary: Option<&str>, error: Option<&str>, now: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query(
        "UPDATE board_cards SET status = ?, outcome = ?, ai_summary = COALESCE(?, ai_summary), error_message = ?, finished_at = ?, claim_expires_at = NULL, updated_at = ?, version = version + 1
         WHERE id = ? AND status = ? AND claimed_by = ?",
    )
    .bind(DONE).bind(outcome).bind(summary).bind(error).bind(now).bind(now).bind(id).bind(RUNNING).bind(who)
    .execute(pool).await?;
    Ok(r.rows_affected() == 1)
}

/// 處理中更新摘要（領取者）。
pub async fn set_summary(pool: &SqlitePool, id: &str, who: &str, summary: &str) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE board_cards SET ai_summary = ?, updated_at = ?, version = version + 1 WHERE id = ? AND status = ? AND claimed_by = ?").bind(summary).bind(now()).bind(id).bind(RUNNING).bind(who).execute(pool).await?;
    Ok(r.rows_affected() == 1)
}

/// 租約逾期的處理中卡片放回待派送（領取者關機、當機或斷線）。回傳收回的張數。
pub async fn expire_claims(pool: &SqlitePool, now: i64) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("UPDATE board_cards SET status = ?, claimed_by = NULL, claimed_at = NULL, claim_expires_at = NULL, updated_at = ?, version = version + 1 WHERE status = ? AND claim_expires_at IS NOT NULL AND claim_expires_at < ?")
        .bind(QUEUED).bind(now).bind(RUNNING).bind(now).execute(pool).await?;
    Ok(r.rows_affected())
}

/// server 重啟時：它自己領的處理中卡片，行程已經沒了，立刻放回待派送，不必等租約逾期。成員領的不動。
pub async fn release_claims_by(pool: &SqlitePool, who: &str, now: i64) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("UPDATE board_cards SET status = ?, claimed_by = NULL, claimed_at = NULL, claim_expires_at = NULL, error_message = ?, updated_at = ?, version = version + 1 WHERE status = ? AND claimed_by = ?")
        .bind(QUEUED).bind("server 重啟，這張卡重新開始").bind(now).bind(RUNNING).bind(who).execute(pool).await?;
    Ok(r.rows_affected())
}

// ---- 附件與留言 ----

pub async fn add_attachment(pool: &SqlitePool, card_id: &str, a: &crate::attach::Saved) -> Result<BoardAttachment, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO board_attachments (id, card_id, filename, path, mime, size) VALUES (?,?,?,?,?,?)").bind(&id).bind(card_id).bind(&a.filename).bind(&a.path).bind(a.mime).bind(a.size).execute(pool).await?;
    sqlx::query("UPDATE board_cards SET updated_at = ?, version = version + 1 WHERE id = ?").bind(now()).bind(card_id).execute(pool).await?;
    Ok(sqlx::query_as("SELECT * FROM board_attachments WHERE id = ?").bind(&id).fetch_one(pool).await?)
}

pub async fn list_attachments(pool: &SqlitePool, card_id: &str) -> Result<Vec<BoardAttachment>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_attachments WHERE card_id = ? ORDER BY filename").bind(card_id).fetch_all(pool).await
}

pub async fn get_attachment(pool: &SqlitePool, id: &str) -> Result<Option<BoardAttachment>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_attachments WHERE id = ?").bind(id).fetch_optional(pool).await
}

/// 刪附件紀錄，回傳檔案路徑讓呼叫端清掉檔案。
pub async fn remove_attachment(pool: &SqlitePool, id: &str) -> Result<Option<(String, String)>, sqlx::Error> {
    let Some(a) = get_attachment(pool, id).await? else { return Ok(None) };
    sqlx::query("DELETE FROM board_attachments WHERE id = ?").bind(id).execute(pool).await?;
    sqlx::query("UPDATE board_cards SET updated_at = ?, version = version + 1 WHERE id = ?").bind(now()).bind(&a.card_id).execute(pool).await?;
    Ok(Some((a.card_id, a.path)))
}

pub async fn add_comment(pool: &SqlitePool, card_id: &str, author: &str, text: &str) -> Result<BoardComment, sqlx::Error> {
    let r = sqlx::query("INSERT INTO board_comments (card_id, author, text, created_at) VALUES (?,?,?,?)").bind(card_id).bind(author).bind(text.trim()).bind(now()).execute(pool).await?;
    sqlx::query_as("SELECT * FROM board_comments WHERE id = ?").bind(r.last_insert_rowid()).fetch_one(pool).await
}

pub async fn list_comments(pool: &SqlitePool, card_id: &str) -> Result<Vec<BoardComment>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_comments WHERE card_id = ? ORDER BY id").bind(card_id).fetch_all(pool).await
}

// ---- Agent 協作：Agent、指派、記事本 ----

pub const KINDS: [&str; 3] = ["coder", "reviewer", "orchestrator"];

pub struct NewAgent<'a> {
    pub project_id: &'a str,
    pub kind: &'a str,
    pub name: &'a str,
    pub role: &'a str,
    pub prompt_prefix: &'a str,
    pub canvas_x: f64,
    pub canvas_y: f64,
    pub produces_spec: bool,
    pub model: Option<&'a str>,
    pub cli: &'a str,
    pub created_by: &'a str,
}

pub async fn create_agent(pool: &SqlitePool, a: NewAgent<'_>) -> Result<Result<BoardAgent, String>, sqlx::Error> {
    if !KINDS.contains(&a.kind) {
        return Ok(Err(format!("Agent 的種類要是 {}", KINDS.join("、"))));
    }
    if !matches!(a.cli, "claude" | "codex") {
        return Ok(Err("AI 工具要是 claude 或 codex".into()));
    }
    if a.name.trim().is_empty() {
        return Ok(Err("名稱不能是空的".into()));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let t = now();
    sqlx::query("INSERT INTO board_agents (id, project_id, kind, name, role, prompt_prefix, canvas_x, canvas_y, produces_spec, model, cli, created_by, created_at, updated_at, version) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,1)")
        .bind(&id).bind(a.project_id).bind(a.kind).bind(a.name.trim()).bind(a.role).bind(a.prompt_prefix).bind(a.canvas_x).bind(a.canvas_y).bind(a.produces_spec).bind(a.model.map(str::trim).filter(|m| !m.is_empty())).bind(a.cli).bind(a.created_by).bind(t).bind(t)
        .execute(pool).await?;
    Ok(Ok(get_agent(pool, &id).await?.expect("剛建立的 Agent")))
}

pub async fn get_agent(pool: &SqlitePool, id: &str) -> Result<Option<BoardAgent>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_agents WHERE id = ?").bind(id).fetch_optional(pool).await
}

pub async fn list_agents(pool: &SqlitePool, project_id: &str) -> Result<Vec<BoardAgent>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_agents WHERE project_id = ? ORDER BY created_at, id").bind(project_id).fetch_all(pool).await
}

pub struct AgentEdit<'a> {
    pub name: &'a str,
    pub role: &'a str,
    pub prompt_prefix: &'a str,
    pub produces_spec: bool,
    pub model: Option<&'a str>,
    pub cli: &'a str,
}

/// 編輯 Agent。種類建立後不能改（它已經有分支與歷史）。
pub async fn update_agent(pool: &SqlitePool, id: &str, version: i64, e: AgentEdit<'_>) -> Result<Result<BoardAgent, AgentChange>, sqlx::Error> {
    if get_agent(pool, id).await?.is_none() {
        return Ok(Err(AgentChange::NotFound));
    }
    if e.name.trim().is_empty() || !matches!(e.cli, "claude" | "codex") {
        return Ok(Err(AgentChange::Refused("名稱不能是空的，AI 工具要是 claude 或 codex".into())));
    }
    let r = sqlx::query("UPDATE board_agents SET name = ?, role = ?, prompt_prefix = ?, produces_spec = ?, model = ?, cli = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?")
        .bind(e.name.trim()).bind(e.role).bind(e.prompt_prefix).bind(e.produces_spec).bind(e.model.map(str::trim).filter(|m| !m.is_empty())).bind(e.cli).bind(now()).bind(id).bind(version)
        .execute(pool).await?;
    if r.rows_affected() == 0 {
        return Ok(Err(AgentChange::Conflict));
    }
    Ok(Ok(get_agent(pool, id).await?.expect("剛更新的 Agent")))
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentChange {
    Conflict,
    NotFound,
    Refused(String),
}

/// 拖動節點只是改位置：不帶版本、不算內容變動（多人同時拖不同節點不該互相擋）。
pub async fn set_agent_position(pool: &SqlitePool, id: &str, x: f64, y: f64) -> Result<bool, sqlx::Error> {
    if !x.is_finite() || !y.is_finite() {
        return Ok(false);
    }
    let r = sqlx::query("UPDATE board_agents SET canvas_x = ?, canvas_y = ? WHERE id = ?").bind(x).bind(y).bind(id).execute(pool).await?;
    Ok(r.rows_affected() == 1)
}

/// 刪除 Agent：它名下處理中的卡片還在跑就不能刪；其餘卡片取消指派、退回計畫中。回傳 `Err(原因)` 表示不能刪。
pub async fn delete_agent(pool: &SqlitePool, id: &str) -> Result<Result<bool, String>, sqlx::Error> {
    let running: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_cards WHERE agent_id = ? AND status = ?").bind(id).bind(RUNNING).fetch_one(pool).await?;
    if running > 0 {
        return Ok(Err("這個 Agent 還有處理中的卡片，請等它做完或先放回".into()));
    }
    sqlx::query("UPDATE board_cards SET agent_id = NULL, status = ?, updated_at = ?, version = version + 1 WHERE agent_id = ? AND status != ?").bind(PLANNING).bind(now()).bind(id).bind(DONE).execute(pool).await?;
    sqlx::query("UPDATE board_cards SET agent_id = NULL WHERE agent_id = ?").bind(id).execute(pool).await?;
    let r = sqlx::query("DELETE FROM board_agents WHERE id = ?").bind(id).execute(pool).await?;
    Ok(Ok(r.rows_affected() == 1))
}

/// 指派卡片給 Agent（排進它的佇列＝待派送），或取消指派（`None`，退回計畫中）。只有計畫中與待派送的卡片能改。
pub async fn assign_card(pool: &SqlitePool, card_id: &str, version: i64, agent_id: Option<&str>) -> Result<Change, sqlx::Error> {
    let Some(cur) = get_card(pool, card_id).await? else { return Ok(Change::NotFound) };
    if cur.status != PLANNING && cur.status != QUEUED {
        return Ok(Change::Refused("只有計畫中與待派送的卡片可以改指派".into()));
    }
    let mut is_spec = false;
    let to_status = match agent_id {
        Some(a) => {
            match get_agent(pool, a).await? {
                None => return Ok(Change::Refused("找不到這個 Agent".into())),
                Some(ag) if ag.project_id != cur.project_id => return Ok(Change::Refused("Agent 與卡片不在同一個專案".into())),
                // 指派給「產出規格」的 Coder：這張是規格卡，通過審查後交給 Orchestrator 拆解。
                Some(ag) => is_spec = ag.produces_spec,
            }
            QUEUED
        }
        None => PLANNING,
    };
    let order = next_order(pool, &cur.project_id, to_status).await?;
    let r = sqlx::query("UPDATE board_cards SET agent_id = ?, status = ?, sort_order = ?, is_spec = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?")
        .bind(agent_id).bind(to_status).bind(order).bind(is_spec).bind(now()).bind(card_id).bind(version).execute(pool).await?;
    if r.rows_affected() == 0 {
        return Ok(Change::Conflict);
    }
    Ok(Change::Ok(Box::new(get_card(pool, card_id).await?.expect("剛指派的卡片"))))
}

pub async fn add_note(pool: &SqlitePool, project_id: &str, author: &str, text: &str) -> Result<BoardNote, sqlx::Error> {
    let r = sqlx::query("INSERT INTO board_notes (project_id, author, text, created_at) VALUES (?,?,?,?)").bind(project_id).bind(author).bind(text.trim()).bind(now()).execute(pool).await?;
    sqlx::query_as("SELECT * FROM board_notes WHERE id = ?").bind(r.last_insert_rowid()).fetch_one(pool).await
}

pub async fn list_notes(pool: &SqlitePool, project_id: &str) -> Result<Vec<BoardNote>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_notes WHERE project_id = ? ORDER BY id").bind(project_id).fetch_all(pool).await
}

pub async fn delete_note(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM board_notes WHERE id = ?").bind(id).execute(pool).await?.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        init_schema(&p).await.unwrap();
        p
    }

    async fn card(p: &SqlitePool, title: &str) -> BoardCard {
        create_card(p, NewCard { project_id: "P", title, body: "內容", parallel_ok: true, interactive: false, label: None, created_by: "小明" }).await.unwrap()
    }

    fn edit<'a>(title: &'a str) -> Edit<'a> {
        Edit { title, body: "新內容", parallel_ok: false, interactive: true, label: Some(" 急 ") }
    }

    #[tokio::test]
    async fn a_new_card_starts_in_planning_at_the_bottom_with_version_one() {
        let p = pool().await;
        let (a, b) = (card(&p, "一").await, card(&p, "二").await);
        assert_eq!((a.status.as_str(), a.version, a.created_by.as_str(), a.claimed_by.clone()), (PLANNING, 1, "小明", None));
        assert!(b.sort_order > a.sort_order, "新卡片排在欄位最下面");
        let list = list_cards(&p, "P", false).await.unwrap();
        assert_eq!(list.iter().map(|c| c.card.title.as_str()).collect::<Vec<_>>(), vec!["一", "二"]);
        assert!(list_cards(&p, "別的專案", false).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_edit_with_a_stale_version_is_a_conflict_and_changes_nothing() {
        let p = pool().await;
        let c = card(&p, "原標題").await;
        let Change::Ok(v2) = update_card(&p, &c.id, 1, edit("甲改的")).await.unwrap() else { panic!("第一個編輯要成功") };
        assert_eq!((v2.version, v2.title.as_str(), v2.label.as_deref(), v2.parallel_ok, v2.interactive), (2, "甲改的", Some("急"), false, true));
        // 乙也是看到版本 1 時開始編輯。
        assert_eq!(update_card(&p, &c.id, 1, edit("乙改的")).await.unwrap(), Change::Conflict);
        assert_eq!(get_card(&p, &c.id).await.unwrap().unwrap().title, "甲改的", "不能靜默覆蓋別人的修改");
        assert_eq!(update_card(&p, "沒有", 1, edit("x")).await.unwrap(), Change::NotFound);
        assert!(matches!(update_card(&p, &c.id, 2, edit("  ")).await.unwrap(), Change::Refused(_)), "標題不能空");
    }

    #[tokio::test]
    async fn moving_is_only_between_planning_and_queued_and_orders_by_neighbours() {
        let p = pool().await;
        let (a, b, c) = (card(&p, "a").await, card(&p, "b").await, card(&p, "c").await);
        // a、b、c 都移到 queued，順序 c、a、b：c 放最上，a 放在 c 之後，b 放在 a 之後。
        let Change::Ok(c) = move_card(&p, &c.id, 1, QUEUED, None, None).await.unwrap() else { panic!() };
        let Change::Ok(a) = move_card(&p, &a.id, 1, QUEUED, Some(&c.id), None).await.unwrap() else { panic!() };
        let Change::Ok(b) = move_card(&p, &b.id, 1, QUEUED, Some(&a.id), None).await.unwrap() else { panic!() };
        let titles: Vec<String> = list_cards(&p, "P", false).await.unwrap().into_iter().map(|v| v.card.title).collect();
        assert_eq!(titles, vec!["c", "a", "b"]);
        // 把 b 插到 c 與 a 之間。
        let Change::Ok(_) = move_card(&p, &b.id, b.version, QUEUED, Some(&c.id), Some(&a.id)).await.unwrap() else { panic!() };
        let titles: Vec<String> = list_cards(&p, "P", false).await.unwrap().into_iter().map(|v| v.card.title).collect();
        assert_eq!(titles, vec!["c", "b", "a"]);
        assert!(matches!(move_card(&p, &c.id, 99, PLANNING, None, None).await.unwrap(), Change::Conflict));
        assert!(matches!(move_card(&p, &c.id, c.version, DONE, None, None).await.unwrap(), Change::Refused(_)), "不能手動移到完成");
        assert!(matches!(move_card(&p, &c.id, c.version, RUNNING, None, None).await.unwrap(), Change::Refused(_)), "不能手動移到執行中");
    }

    #[tokio::test]
    async fn an_explicit_sort_order_is_used_as_given() {
        let p = pool().await;
        let c = card(&p, "x").await;
        let Change::Ok(m) = move_card_at(&p, &c.id, 1, QUEUED, None, None, Some(7.5)).await.unwrap() else { panic!() };
        assert_eq!((m.status.as_str(), m.sort_order), (QUEUED, 7.5));
        let Change::Ok(n) = move_card_at(&p, &c.id, m.version, QUEUED, None, None, Some(f64::NAN)).await.unwrap() else { panic!() };
        assert!(n.sort_order.is_finite(), "不合法的排序值退回用鄰居算");
    }

    #[tokio::test]
    async fn only_a_queued_card_can_be_claimed_once_and_only_by_the_claimer_can_it_finish() {
        let p = pool().await;
        let c = card(&p, "x").await;
        assert!(!claim_card(&p, &c.id, "小明", 100).await.unwrap(), "計畫中的不能領");
        let Change::Ok(q) = move_card(&p, &c.id, 1, QUEUED, None, None).await.unwrap() else { panic!() };
        assert!(claim_card(&p, &q.id, "小明", 100).await.unwrap());
        assert!(!claim_card(&p, &q.id, "小華", 101).await.unwrap(), "同一張只有一個人領到");
        let got = get_card(&p, &q.id).await.unwrap().unwrap();
        assert_eq!((got.status.as_str(), got.claimed_by.as_deref(), got.claim_expires_at, got.dispatched_at), (RUNNING, Some("小明"), Some(100 + CLAIM_TTL_SECS), Some(100)));
        assert!(matches!(update_card(&p, &q.id, got.version, edit("改")).await.unwrap(), Change::Refused(_)), "處理中不能編輯");
        assert!(delete_card(&p, &q.id).await.unwrap().is_none(), "處理中不能刪");
        assert!(!finish_claim(&p, &q.id, "小華", "success", None, None, 200).await.unwrap(), "只有領取者能交結果");
        assert!(finish_claim(&p, &q.id, "小明", "success", Some("改好了"), None, 200).await.unwrap());
        let done = get_card(&p, &q.id).await.unwrap().unwrap();
        assert_eq!((done.status.as_str(), done.outcome.as_deref(), done.ai_summary.as_deref(), done.finished_at, done.claimed_by.as_deref()), (DONE, Some("success"), Some("改好了"), Some(200), Some("小明")));
        assert!(!finish_claim(&p, &q.id, "小明", "success", None, None, 201).await.unwrap(), "交過之後不能再交");
    }

    #[tokio::test]
    async fn heartbeat_release_failure_and_expiry() {
        let p = pool().await;
        let c = card(&p, "x").await;
        move_card(&p, &c.id, 1, QUEUED, None, None).await.unwrap();
        claim_card(&p, &c.id, "小明", 100).await.unwrap();
        assert!(!renew_claim(&p, &c.id, "小華", 150).await.unwrap());
        assert!(renew_claim(&p, &c.id, "小明", 150).await.unwrap());
        assert_eq!(get_card(&p, &c.id).await.unwrap().unwrap().claim_expires_at, Some(150 + CLAIM_TTL_SECS));
        assert_eq!(expire_claims(&p, 150 + CLAIM_TTL_SECS).await.unwrap(), 0, "還沒過期");
        assert_eq!(expire_claims(&p, 151 + CLAIM_TTL_SECS).await.unwrap(), 1);
        let back = get_card(&p, &c.id).await.unwrap().unwrap();
        assert_eq!((back.status.as_str(), back.claimed_by), (QUEUED, None), "逾期自動放回待派送");
        claim_card(&p, &c.id, "小華", 300).await.unwrap();
        assert!(!release_claim(&p, &c.id, Some("小明"), 301).await.unwrap(), "別人不能放回");
        assert!(release_claim(&p, &c.id, None, 301).await.unwrap(), "管理員可以放回");
        claim_card(&p, &c.id, "小華", 400).await.unwrap();
        assert!(finish_claim(&p, &c.id, "小華", "failed", None, Some("Claude Code 結束了"), 500).await.unwrap());
        let f = get_card(&p, &c.id).await.unwrap().unwrap();
        assert_eq!((f.outcome.as_deref(), f.error_message.as_deref()), (Some("failed"), Some("Claude Code 結束了")));
    }

    #[tokio::test]
    async fn a_cards_interrupted_conversation_is_remembered_replaced_and_cleared_when_it_finishes() {
        let p = pool().await;
        let c = card(&p, "x").await;
        assert_eq!(get_session(&p, &c.id).await.unwrap(), None);
        set_session(&p, &c.id, "sid-1", Some("abc123")).await.unwrap();
        assert_eq!(get_session(&p, &c.id).await.unwrap(), Some(CardSession { session_id: "sid-1".into(), start_sha: Some("abc123".into()) }));
        set_session(&p, &c.id, "sid-2", None).await.unwrap();
        assert_eq!(get_session(&p, &c.id).await.unwrap().unwrap().session_id, "sid-2", "同一張卡只有一筆，新的取代舊的");
        assert_eq!(clear_session(&p, &c.id).await.unwrap().as_deref(), Some("sid-2"));
        assert_eq!(get_session(&p, &c.id).await.unwrap(), None);
        assert_eq!(clear_session(&p, &c.id).await.unwrap(), None, "沒有可清的不報錯");
    }

    #[tokio::test]
    async fn release_claims_by_only_frees_that_claimers_running_cards() {
        let p = pool().await;
        let (a, b, c) = (card(&p, "a").await, card(&p, "b").await, card(&p, "c").await);
        for x in [&a, &b, &c] {
            move_card(&p, &x.id, 1, QUEUED, None, None).await.unwrap();
        }
        claim_card(&p, &a.id, "server", 100).await.unwrap();
        claim_card(&p, &b.id, "小明", 100).await.unwrap();
        assert_eq!(release_claims_by(&p, "server", 101).await.unwrap(), 1, "只放 server 自己領的");
        let (ra, rb, rc) = (get_card(&p, &a.id).await.unwrap().unwrap(), get_card(&p, &b.id).await.unwrap().unwrap(), get_card(&p, &c.id).await.unwrap().unwrap());
        assert_eq!((ra.status.as_str(), ra.claimed_by), (QUEUED, None));
        assert_eq!((rb.status.as_str(), rb.claimed_by.as_deref()), (RUNNING, Some("小明")), "成員領的不動");
        assert_eq!(rc.status, QUEUED);
    }

    #[tokio::test]
    async fn only_done_cards_can_be_archived_and_come_back() {
        let p = pool().await;
        let a = card(&p, "進行中").await;
        let b = card(&p, "做完").await;
        move_card(&p, &b.id, 1, QUEUED, None, None).await.unwrap();
        claim_card(&p, &b.id, "小明", 1).await.unwrap();
        finish_claim(&p, &b.id, "小明", "success", None, None, 2).await.unwrap();
        assert!(!set_archived(&p, &a.id, true).await.unwrap(), "沒做完的不能封存");
        assert!(set_archived(&p, &b.id, true).await.unwrap());
        assert_eq!(list_cards(&p, "P", false).await.unwrap().len(), 1);
        assert_eq!(list_cards(&p, "P", true).await.unwrap().len(), 1);
        assert!(set_archived(&p, &b.id, false).await.unwrap());
        assert_eq!(archive_all_done(&p, "P").await.unwrap(), 1);
        assert_eq!(archive_all_done(&p, "P").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn attachments_and_comments_belong_to_the_card_and_go_away_with_it() {
        let p = pool().await;
        let c = card(&p, "x").await;
        let saved = crate::attach::Saved { filename: "err.log".into(), path: "/data/err.log".into(), mime: "text/plain", size: 9 };
        let att = add_attachment(&p, &c.id, &saved).await.unwrap();
        add_comment(&p, &c.id, "小華", "  我來看  ").await.unwrap();
        let v = &list_cards(&p, "P", false).await.unwrap()[0];
        assert_eq!((v.attachments.len(), v.comment_count, v.card.version), (1, 1, 2), "新增附件也算一次變動");
        assert_eq!(list_comments(&p, &c.id).await.unwrap()[0].text, "我來看");
        assert_eq!(remove_attachment(&p, &att.id).await.unwrap().map(|x| x.1), Some("/data/err.log".to_string()));
        add_attachment(&p, &c.id, &saved).await.unwrap();
        assert_eq!(delete_card(&p, &c.id).await.unwrap(), Some(vec!["/data/err.log".to_string()]));
        assert!(get_card(&p, &c.id).await.unwrap().is_none() && list_comments(&p, &c.id).await.unwrap().is_empty());
    }

    fn agent<'a>(kind: &'a str, name: &'a str) -> NewAgent<'a> {
        NewAgent { project_id: "P", kind, name, role: "Senior Coder", prompt_prefix: "", canvas_x: 10.0, canvas_y: 20.0, produces_spec: false, model: Some(" opus "), cli: "claude", created_by: "小明" }
    }

    #[tokio::test]
    async fn agents_are_validated_listed_per_project_and_edited_with_version_checks() {
        let p = pool().await;
        assert!(create_agent(&p, agent("boss", "x")).await.unwrap().is_err(), "種類要合法");
        assert!(create_agent(&p, agent("coder", "  ")).await.unwrap().is_err(), "名稱不能空");
        assert!(create_agent(&p, NewAgent { cli: "gpt", ..agent("coder", "x") }).await.unwrap().is_err());
        let a = create_agent(&p, agent("coder", "Coder A")).await.unwrap().unwrap();
        assert_eq!((a.kind.as_str(), a.model.as_deref(), a.version, a.cli.as_str()), ("coder", Some("opus"), 1, "claude"));
        create_agent(&p, NewAgent { project_id: "別的專案", ..agent("reviewer", "R") }).await.unwrap().unwrap();
        assert_eq!(list_agents(&p, "P").await.unwrap().len(), 1, "每個專案各自的 Agent");

        let edit = |n: &'static str| AgentEdit { name: n, role: "Lead", prompt_prefix: "多寫測試", produces_spec: true, model: None, cli: "codex" };
        let b = update_agent(&p, &a.id, 1, edit("改名")).await.unwrap().unwrap();
        assert_eq!((b.name.as_str(), b.version, b.cli.as_str(), b.model, b.produces_spec, b.kind.as_str()), ("改名", 2, "codex", None, true, "coder"), "種類不能改");
        assert_eq!(update_agent(&p, &a.id, 1, edit("過期的")).await.unwrap().unwrap_err(), AgentChange::Conflict);
        assert_eq!(update_agent(&p, "沒有", 1, edit("x")).await.unwrap().unwrap_err(), AgentChange::NotFound);
        assert!(set_agent_position(&p, &a.id, 300.0, 400.0).await.unwrap());
        assert!(!set_agent_position(&p, &a.id, f64::NAN, 1.0).await.unwrap());
        let moved = get_agent(&p, &a.id).await.unwrap().unwrap();
        assert_eq!((moved.canvas_x, moved.canvas_y, moved.version), (300.0, 400.0, 2), "拖動節點不算內容變動");
    }

    #[tokio::test]
    async fn a_card_given_to_a_spec_writer_is_a_spec_and_stops_being_one_when_moved_to_another_coder() {
        let p = pool().await;
        let writer = create_agent(&p, NewAgent { produces_spec: true, ..agent("coder", "規格站") }).await.unwrap().unwrap();
        let plain = create_agent(&p, agent("coder", "工程師")).await.unwrap().unwrap();
        let c = card(&p, "x").await;
        let Change::Ok(q) = assign_card(&p, &c.id, 1, Some(&writer.id)).await.unwrap() else { panic!() };
        assert!(q.is_spec);
        let Change::Ok(back) = assign_card(&p, &q.id, q.version, None).await.unwrap() else { panic!() };
        assert!(!back.is_spec, "退回計畫中就不是規格了");
        let Change::Ok(q2) = assign_card(&p, &back.id, back.version, Some(&plain.id)).await.unwrap() else { panic!() };
        assert!(!q2.is_spec);
    }

    #[tokio::test]
    async fn workflow_cards_keep_their_dependencies_and_the_spec_they_came_from() {
        let p = pool().await;
        let w = create_workflow_card(&p, NewWorkflowCard { project_id: "p1", title: "資料層", body: "b", kind: "work", agent_id: None, module_id: None, review_of: None, status: QUEUED, depends_on: Some("[\"x\"]"), from_spec: Some("spec1") }).await.unwrap();
        let got = get_card(&p, &w.id).await.unwrap().unwrap();
        assert_eq!((got.card_kind.as_str(), got.depends_on.as_deref(), got.from_spec.as_deref(), got.status.as_str()), ("work", Some("[\"x\"]"), Some("spec1"), QUEUED));
    }

    #[tokio::test]
    async fn assigning_queues_the_card_for_the_agent_and_unassigning_returns_it_to_planning() {
        let p = pool().await;
        let a = create_agent(&p, agent("coder", "A")).await.unwrap().unwrap();
        let c = card(&p, "x").await;
        let Change::Ok(q) = assign_card(&p, &c.id, 1, Some(&a.id)).await.unwrap() else { panic!() };
        assert_eq!((q.status.as_str(), q.agent_id.as_deref(), q.version), (QUEUED, Some(a.id.as_str()), 2));
        assert_eq!(assign_card(&p, &c.id, 1, None).await.unwrap(), Change::Conflict, "過期版本");
        assert!(matches!(assign_card(&p, &c.id, 2, Some("沒有")).await.unwrap(), Change::Refused(_)));
        let other = create_agent(&p, NewAgent { project_id: "別的專案", ..agent("coder", "B") }).await.unwrap().unwrap();
        assert!(matches!(assign_card(&p, &c.id, 2, Some(&other.id)).await.unwrap(), Change::Refused(m) if m.contains("同一個專案")));
        let Change::Ok(back) = assign_card(&p, &c.id, 2, None).await.unwrap() else { panic!() };
        assert_eq!((back.status.as_str(), back.agent_id), (PLANNING, None));
        // 處理中的卡片不能改指派。
        let Change::Ok(q) = assign_card(&p, &c.id, back.version, Some(&a.id)).await.unwrap() else { panic!() };
        claim_card(&p, &q.id, "小明", 1).await.unwrap();
        let running = get_card(&p, &q.id).await.unwrap().unwrap();
        assert!(matches!(assign_card(&p, &q.id, running.version, None).await.unwrap(), Change::Refused(_)));
    }

    #[tokio::test]
    async fn deleting_an_agent_frees_its_cards_unless_one_is_running() {
        let p = pool().await;
        let a = create_agent(&p, agent("coder", "A")).await.unwrap().unwrap();
        let (c1, c2) = (card(&p, "排隊中").await, card(&p, "處理中").await);
        assign_card(&p, &c1.id, 1, Some(&a.id)).await.unwrap();
        let Change::Ok(q2) = assign_card(&p, &c2.id, 1, Some(&a.id)).await.unwrap() else { panic!() };
        claim_card(&p, &q2.id, "小明", 1).await.unwrap();
        assert!(delete_agent(&p, &a.id).await.unwrap().unwrap_err().contains("處理中"), "有卡片在跑不能刪");
        release_claim(&p, &q2.id, None, 2).await.unwrap();
        assert_eq!(delete_agent(&p, &a.id).await.unwrap(), Ok(true));
        for id in [&c1.id, &c2.id] {
            let c = get_card(&p, id).await.unwrap().unwrap();
            assert_eq!((c.agent_id, c.status.as_str()), (None, PLANNING), "卡片取消指派、退回計畫中");
        }
    }

    #[tokio::test]
    async fn agent_collaboration_columns_exist_on_old_databases_and_notes_are_per_project() {
        // 舊資料庫：只有 M1 的欄位。升級後讀寫 Agent 欄位不能壞。
        let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE board_cards (id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'planning', sort_order REAL NOT NULL DEFAULT 0, label TEXT, parallel_ok INTEGER NOT NULL DEFAULT 1, interactive INTEGER NOT NULL DEFAULT 0, outcome TEXT, ai_summary TEXT, error_message TEXT, created_by TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, dispatched_at INTEGER, finished_at INTEGER, archived_at INTEGER, claimed_by TEXT, claimed_at INTEGER, claim_expires_at INTEGER, version INTEGER NOT NULL DEFAULT 1)").execute(&p).await.unwrap();
        sqlx::query("INSERT INTO board_cards (id, project_id, title, created_at, updated_at) VALUES ('old','P','舊卡片',1,1)").execute(&p).await.unwrap();
        init_schema(&p).await.unwrap();
        init_schema(&p).await.unwrap(); // 可重複執行
        let c = get_card(&p, "old").await.unwrap().unwrap();
        assert_eq!((c.card_kind.as_str(), c.agent_id, c.module_override, c.is_spec), ("work", None, false, false), "舊卡片補上預設值");
        let n1 = add_note(&p, "P", "", "  先做登入  ").await.unwrap();
        add_note(&p, "別人", "Agent A", "x").await.unwrap();
        assert_eq!(list_notes(&p, "P").await.unwrap(), vec![n1.clone()]);
        assert_eq!(n1.text, "先做登入");
        assert!(delete_note(&p, n1.id).await.unwrap() && !delete_note(&p, n1.id).await.unwrap());
    }
}
