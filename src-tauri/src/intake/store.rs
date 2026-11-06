//! `intake.db`：問題佇列、白名單、設定、丟棄計數。自由函式 + `&SqlitePool`，與 `tasks/store.rs` 同風格。
//!
//! 狀態：`pending`（等你按派送）→ `queued`（可被接收端領取）→ `running` →
//! `done` 或 `draft`（外部回覆待寄出）→ `done`；`rejected` 為終態。
use super::rules::{Level, Sender};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

pub const PENDING: &str = "pending";
pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const DONE: &str = "done";
pub const DRAFT: &str = "draft";
pub const REJECTED: &str = "rejected";

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct IntakeItem {
    pub id: String,
    pub source: String,
    pub sender: String,
    pub sender_level: String,
    pub subject: String,
    pub body: String,
    pub message_id: String,
    pub status: String,
    pub tab_id: Option<String>,
    pub reply_draft: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// `question`／`code_change`；尚未分類為 `None`。
    pub kind: Option<String>,
    /// 派工前的 git HEAD，驗證時用來算「這次改了什麼」。
    pub snapshot_head: Option<String>,
    /// 驗證結果的文字報告（也會附在回信裡）。
    pub verification: Option<String>,
    pub verified: Option<bool>,
    /// 提問者的顯示名稱（信件的寄件人名稱、Telegram 的姓名）；沒有就是 `None`。
    pub sender_name: Option<String>,
    /// 歸屬的專案（看板專案 id）。空字串＝尚未歸屬（升級前的舊資料）。
    pub project_id: String,
}

pub struct NewItem<'a> {
    pub project_id: &'a str,
    /// 先決定好 id（附圖要先依 id 存檔）；`None` 就自動產生。
    pub id: Option<&'a str>,
    pub source: &'a str,
    pub sender: &'a str,
    pub level: Level,
    pub subject: &'a str,
    pub body: &'a str,
    pub message_id: &'a str,
    /// 收進來時的起始狀態：自動派送的層級傳 `QUEUED`，手動的傳 `PENDING`。
    pub status: &'a str,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn level_str(l: Level) -> &'static str {
    match l {
        Level::Internal => "internal",
        Level::External => "external",
    }
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    for q in [
        "CREATE TABLE IF NOT EXISTS intake_items (
            id TEXT PRIMARY KEY NOT NULL,
            source TEXT NOT NULL,
            sender TEXT NOT NULL,
            sender_level TEXT NOT NULL,
            subject TEXT NOT NULL,
            body TEXT NOT NULL,
            message_id TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL,
            tab_id TEXT,
            reply_draft TEXT,
            error TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
        "CREATE INDEX IF NOT EXISTS idx_intake_status ON intake_items(status, created_at)",
        "CREATE TABLE IF NOT EXISTS intake_attachments (
            id TEXT PRIMARY KEY NOT NULL,
            item_id TEXT NOT NULL,
            filename TEXT NOT NULL,
            path TEXT NOT NULL,
            mime TEXT NOT NULL,
            size INTEGER NOT NULL
        )",
        "CREATE INDEX IF NOT EXISTS idx_intake_att_item ON intake_attachments(item_id)",
        "CREATE TABLE IF NOT EXISTS intake_senders (
            address TEXT PRIMARY KEY NOT NULL,
            level TEXT NOT NULL,
            skip_auth INTEGER NOT NULL DEFAULT 0
        )",
        "CREATE TABLE IF NOT EXISTS intake_settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS intake_dropped (reason TEXT PRIMARY KEY NOT NULL, count INTEGER NOT NULL DEFAULT 0)",
        // 依專案分流（第 5 期）：白名單與丟棄計數改成以專案為單位。舊表保留不動，只複製一次。
        "CREATE TABLE IF NOT EXISTS intake_project_senders (
            project_id TEXT NOT NULL,
            address TEXT NOT NULL,
            level TEXT NOT NULL,
            skip_auth INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (project_id, address)
        )",
        "CREATE TABLE IF NOT EXISTS intake_project_dropped (
            project_id TEXT NOT NULL,
            reason TEXT NOT NULL,
            count INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (project_id, reason)
        )",
    ] {
        sqlx::query(q).execute(pool).await?;
    }
    // 第 2 期新增的欄位。CREATE 是 IF NOT EXISTS，舊資料庫不會自動長出來，逐欄補。
    let have: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('intake_items')").fetch_all(pool).await?;
    for (col, ty) in [("kind", "TEXT"), ("snapshot_head", "TEXT"), ("verification", "TEXT"), ("verified", "INTEGER"), ("sender_name", "TEXT")] {
        if !have.iter().any(|c| c == col) {
            sqlx::query(&format!("ALTER TABLE intake_items ADD COLUMN {col} {ty}")).execute(pool).await?;
        }
    }
    if !have.iter().any(|c| c == "project_id") {
        sqlx::query("ALTER TABLE intake_items ADD COLUMN project_id TEXT NOT NULL DEFAULT ''").execute(pool).await?;
    }
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_intake_project_status ON intake_items(project_id, status, created_at)").execute(pool).await?;
    // 舊的單一白名單／丟棄計數搬進「未歸屬」（project_id = ''），只做一次，之後使用者刪掉的不會再長回來。
    if get_setting(pool, "project_split_migrated").await?.is_none() {
        sqlx::query("INSERT OR IGNORE INTO intake_project_senders (project_id, address, level, skip_auth) SELECT '', address, level, skip_auth FROM intake_senders").execute(pool).await?;
        sqlx::query("INSERT OR IGNORE INTO intake_project_dropped (project_id, reason, count) SELECT '', reason, count FROM intake_dropped").execute(pool).await?;
        set_setting(pool, "project_split_migrated", "1").await?;
    }
    Ok(())
}

/// 顯示名稱來自外部，清掉控制字元、截短；空白或跟地址一樣就當作沒有。
pub fn clean_name(name: &str, address: &str) -> Option<String> {
    let n: String = name.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(80).collect();
    let n = n.trim().to_string();
    if n.is_empty() || n.eq_ignore_ascii_case(address) { None } else { Some(n) }
}

pub async fn set_sender_name(pool: &SqlitePool, id: &str, name: &str, address: &str) -> Result<(), sqlx::Error> {
    if let Some(n) = clean_name(name, address) {
        sqlx::query("UPDATE intake_items SET sender_name = ? WHERE id = ?").bind(n).bind(id).execute(pool).await?;
    }
    Ok(())
}

pub async fn set_kind(pool: &SqlitePool, id: &str, kind: &str, snapshot_head: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE intake_items SET kind = ?, snapshot_head = ?, updated_at = ? WHERE id = ?")
        .bind(kind).bind(snapshot_head).bind(now()).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn set_verification(pool: &SqlitePool, id: &str, report: &str, passed: bool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE intake_items SET verification = ?, verified = ?, updated_at = ? WHERE id = ?")
        .bind(report).bind(passed).bind(now()).bind(id).execute(pool).await?;
    Ok(())
}

/// 以 `message_id` 去重：已存在回 `None`。
pub async fn insert_item(pool: &SqlitePool, n: NewItem<'_>) -> Result<Option<IntakeItem>, sqlx::Error> {
    let id = n.id.map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let t = now();
    let r = sqlx::query(
        "INSERT OR IGNORE INTO intake_items
         (id, source, sender, sender_level, subject, body, message_id, status, created_at, updated_at, project_id)
         VALUES (?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id).bind(n.source).bind(n.sender).bind(level_str(n.level)).bind(n.subject)
    .bind(n.body).bind(n.message_id).bind(n.status).bind(t).bind(t).bind(n.project_id)
    .execute(pool).await?;
    if r.rows_affected() == 0 {
        return Ok(None);
    }
    get_item(pool, &id).await
}

pub async fn get_item(pool: &SqlitePool, id: &str) -> Result<Option<IntakeItem>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM intake_items WHERE id = ?").bind(id).fetch_optional(pool).await
}

/// 某專案的題目（新的在前）。`project_id` 為空字串＝未歸屬。
pub async fn list_items(pool: &SqlitePool, project_id: &str, status: Option<&str>) -> Result<Vec<IntakeItem>, sqlx::Error> {
    match status {
        Some(s) => sqlx::query_as("SELECT * FROM intake_items WHERE project_id = ? AND status = ? ORDER BY created_at DESC, rowid DESC").bind(project_id).bind(s).fetch_all(pool).await,
        None => sqlx::query_as("SELECT * FROM intake_items WHERE project_id = ? ORDER BY created_at DESC, rowid DESC").bind(project_id).fetch_all(pool).await,
    }
}

async fn transition(pool: &SqlitePool, id: &str, from: &[&str], to: &str) -> Result<bool, sqlx::Error> {
    let ph = from.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("UPDATE intake_items SET status = ?, error = NULL, updated_at = ? WHERE id = ? AND status IN ({ph})");
    let mut q = sqlx::query(&sql).bind(to).bind(now()).bind(id);
    for f in from {
        q = q.bind(*f);
    }
    Ok(q.execute(pool).await?.rows_affected() > 0)
}

/// 使用者按「派送」：`pending → queued`。
pub async fn approve(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    transition(pool, id, &[PENDING], QUEUED).await
}

pub async fn reject(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    transition(pool, id, &[PENDING, QUEUED], REJECTED).await
}

/// 刪除一題（含附圖紀錄）。處理中（`running`）的不能刪：Agent 正在做，刪了回覆會無處可存。
pub async fn delete_item(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("DELETE FROM intake_items WHERE id = ? AND status != ?").bind(id).bind(RUNNING).execute(pool).await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query("DELETE FROM intake_attachments WHERE item_id = ?").bind(id).execute(pool).await?;
    Ok(true)
}

/// 原子領取最舊的 `queued`，標成 `running` 並記下分頁。
pub async fn claim_next(pool: &SqlitePool, project_id: &str, tab_id: &str) -> Result<Option<IntakeItem>, sqlx::Error> {
    let r = sqlx::query(
        "UPDATE intake_items SET status = ?, tab_id = ?, error = NULL, updated_at = ?
         WHERE id = (SELECT id FROM intake_items WHERE status = ? AND project_id = ? ORDER BY created_at, rowid LIMIT 1)
           AND status = ?",
    )
    .bind(RUNNING).bind(tab_id).bind(now()).bind(QUEUED).bind(project_id).bind(QUEUED)
    .execute(pool).await?;
    if r.rows_affected() == 0 {
        return Ok(None);
    }
    sqlx::query_as("SELECT * FROM intake_items WHERE status = ? AND tab_id = ? ORDER BY updated_at DESC, rowid DESC LIMIT 1")
        .bind(RUNNING).bind(tab_id).fetch_optional(pool).await
}

/// 完成並存回覆。`auto_send` 為真→`done`（呼叫端隨後寄出）；否則→`draft`。
pub async fn finish(pool: &SqlitePool, id: &str, reply: &str, auto_send: bool) -> Result<bool, sqlx::Error> {
    let to = if auto_send { DONE } else { DRAFT };
    let r = sqlx::query("UPDATE intake_items SET status = ?, reply_draft = ?, error = NULL, updated_at = ? WHERE id = ? AND status = ?")
        .bind(to).bind(reply).bind(now()).bind(id).bind(RUNNING)
        .execute(pool).await?;
    Ok(r.rows_affected() > 0)
}

/// 失敗或分頁關閉：回 `pending` 並記原因，不自動重派（避免無限重試）。
pub async fn fail(pool: &SqlitePool, id: &str, error: &str) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE intake_items SET status = ?, tab_id = NULL, error = ?, updated_at = ? WHERE id = ? AND status = ?")
        .bind(PENDING).bind(error).bind(now()).bind(id).bind(RUNNING)
        .execute(pool).await?;
    Ok(r.rows_affected() > 0)
}

pub async fn update_draft(pool: &SqlitePool, id: &str, draft: &str) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE intake_items SET reply_draft = ?, updated_at = ? WHERE id = ? AND status = ?")
        .bind(draft).bind(now()).bind(id).bind(DRAFT).execute(pool).await?;
    Ok(r.rows_affected() > 0)
}

/// 寄信失敗等：只記原因，不改狀態。
pub async fn set_error(pool: &SqlitePool, id: &str, error: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE intake_items SET error = ?, updated_at = ? WHERE id = ?").bind(error).bind(now()).bind(id).execute(pool).await?;
    Ok(())
}

/// 草稿寄出後：`draft → done`。
pub async fn mark_sent(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    transition(pool, id, &[DRAFT], DONE).await
}

/// 啟動時：上次中斷留下的 `running` 一律退回 `pending`。
pub async fn recover_interrupted(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("UPDATE intake_items SET status = ?, tab_id = NULL, error = ?, updated_at = ? WHERE status = ?")
        .bind(PENDING).bind("應用程式重新啟動，工作中斷").bind(now()).bind(RUNNING)
        .execute(pool).await?;
    Ok(r.rows_affected())
}

pub async fn count_from_sender_since(pool: &SqlitePool, project_id: &str, sender: &str, since: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM intake_items WHERE project_id = ? AND sender = ? AND created_at >= ?")
        .bind(project_id).bind(sender).bind(since).fetch_one(pool).await
}

// ---- 附圖 ----

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct AttachmentRow {
    pub id: String,
    pub item_id: String,
    pub filename: String,
    pub path: String,
    pub mime: String,
    pub size: i64,
}

pub async fn add_attachment(pool: &SqlitePool, item_id: &str, a: &super::attach::Saved) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO intake_attachments (id, item_id, filename, path, mime, size) VALUES (?,?,?,?,?,?)")
        .bind(uuid::Uuid::new_v4().to_string()).bind(item_id).bind(&a.filename).bind(&a.path).bind(a.mime).bind(a.size)
        .execute(pool).await?;
    Ok(())
}

pub async fn list_attachments(pool: &SqlitePool, item_id: &str) -> Result<Vec<AttachmentRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM intake_attachments WHERE item_id = ? ORDER BY filename").bind(item_id).fetch_all(pool).await
}

pub async fn get_attachment(pool: &SqlitePool, id: &str) -> Result<Option<AttachmentRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM intake_attachments WHERE id = ?").bind(id).fetch_optional(pool).await
}

// ---- 白名單 ----

#[derive(Debug, Clone, FromRow)]
struct SenderRow {
    address: String,
    level: String,
    skip_auth: bool,
}

pub async fn upsert_sender(pool: &SqlitePool, project_id: &str, s: &Sender) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO intake_project_senders (project_id, address, level, skip_auth) VALUES (?,?,?,?)
                 ON CONFLICT(project_id, address) DO UPDATE SET level = excluded.level, skip_auth = excluded.skip_auth")
        .bind(project_id).bind(super::rules::normalize(&s.address)).bind(level_str(s.level)).bind(s.skip_auth)
        .execute(pool).await?;
    Ok(())
}

pub async fn delete_sender(pool: &SqlitePool, project_id: &str, address: &str) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM intake_project_senders WHERE project_id = ? AND address = ?").bind(project_id).bind(super::rules::normalize(address))
        .execute(pool).await?.rows_affected() > 0)
}

pub async fn list_senders(pool: &SqlitePool, project_id: &str) -> Result<Vec<Sender>, sqlx::Error> {
    let rows: Vec<SenderRow> = sqlx::query_as("SELECT address, level, skip_auth FROM intake_project_senders WHERE project_id = ? ORDER BY address").bind(project_id).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| Sender {
        address: r.address,
        level: if r.level == "internal" { Level::Internal } else { Level::External },
        skip_auth: r.skip_auth,
    }).collect())
}

// ---- 設定與丟棄計數 ----

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM intake_settings WHERE key = ?").bind(key).fetch_optional(pool).await
}

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO intake_settings (key, value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key).bind(value).execute(pool).await?;
    Ok(())
}

pub async fn bump_dropped(pool: &SqlitePool, project_id: &str, reason: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO intake_project_dropped (project_id, reason, count) VALUES (?,?,1) ON CONFLICT(project_id, reason) DO UPDATE SET count = count + 1")
        .bind(project_id).bind(reason).execute(pool).await?;
    Ok(())
}

pub async fn dropped_counts(pool: &SqlitePool, project_id: &str) -> Result<Vec<(String, i64)>, sqlx::Error> {
    sqlx::query_as("SELECT reason, count FROM intake_project_dropped WHERE project_id = ? ORDER BY reason").bind(project_id).fetch_all(pool).await
}

// ---- 專案歸屬 ----

/// 各專案「需要你處理」的題數（待派送＋待寄出的草稿）與總數。`project_id` 空字串＝未歸屬。
pub async fn project_counts(pool: &SqlitePool) -> Result<Vec<(String, i64, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT project_id,
                SUM(CASE WHEN status IN ('pending','draft') THEN 1 ELSE 0 END),
                COUNT(*)
         FROM intake_items GROUP BY project_id",
    )
    .fetch_all(pool)
    .await
}

/// 把單一題目指派到專案（只限未歸屬的，避免誤把別的專案的題目搬走）。
pub async fn assign_item(pool: &SqlitePool, id: &str, project_id: &str) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("UPDATE intake_items SET project_id = ?, updated_at = ? WHERE id = ? AND project_id = ''")
        .bind(project_id).bind(now()).bind(id).execute(pool).await?.rows_affected() > 0)
}

/// 把所有未歸屬的題目、白名單、丟棄計數與設定歸給專案（升級後只有一個專案時自動做；多個專案由使用者決定）。
/// 已存在於該專案的白名單項目不覆蓋。回傳搬了幾題。
pub async fn adopt_legacy(pool: &SqlitePool, project_id: &str) -> Result<u64, sqlx::Error> {
    let moved = sqlx::query("UPDATE intake_items SET project_id = ?, updated_at = ? WHERE project_id = ''")
        .bind(project_id).bind(now()).execute(pool).await?.rows_affected();
    sqlx::query("INSERT OR IGNORE INTO intake_project_senders (project_id, address, level, skip_auth) SELECT ?, address, level, skip_auth FROM intake_project_senders WHERE project_id = ''")
        .bind(project_id).execute(pool).await?;
    sqlx::query("DELETE FROM intake_project_senders WHERE project_id = ''").execute(pool).await?;
    sqlx::query("INSERT OR IGNORE INTO intake_project_dropped (project_id, reason, count) SELECT ?, reason, count FROM intake_project_dropped WHERE project_id = ''")
        .bind(project_id).execute(pool).await?;
    sqlx::query("DELETE FROM intake_project_dropped WHERE project_id = ''").execute(pool).await?;
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        init_schema(&p).await.unwrap();
        p
    }
    fn new<'a>(mid: &'a str, status: &'a str) -> NewItem<'a> {
        NewItem { project_id: "", id: None, source: "email", sender: "a@corp.com", level: Level::External, subject: "s", body: "b", message_id: mid, status }
    }

    #[tokio::test]
    async fn duplicate_message_id_is_ignored() {
        let p = pool().await;
        assert!(insert_item(&p, new("m1", PENDING)).await.unwrap().is_some());
        assert!(insert_item(&p, new("m1", PENDING)).await.unwrap().is_none());
        assert_eq!(list_items(&p, "", None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn claim_takes_oldest_queued_only_and_pending_is_never_claimed() {
        let p = pool().await;
        insert_item(&p, new("m0", PENDING)).await.unwrap();
        let a = insert_item(&p, new("m1", QUEUED)).await.unwrap().unwrap();
        insert_item(&p, new("m2", QUEUED)).await.unwrap();
        let got = claim_next(&p, "", "tab1").await.unwrap().unwrap();
        assert_eq!(got.id, a.id);
        assert_eq!(got.status, RUNNING);
        assert_eq!(got.tab_id.as_deref(), Some("tab1"));
        let second = claim_next(&p, "", "tab2").await.unwrap().unwrap();
        assert_eq!(second.message_id, "m2");
        assert!(claim_next(&p, "", "tab3").await.unwrap().is_none(), "pending 不可被領取");
    }

    #[tokio::test]
    async fn manual_approval_moves_pending_to_queued() {
        let p = pool().await;
        let i = insert_item(&p, new("m1", PENDING)).await.unwrap().unwrap();
        assert!(claim_next(&p, "", "t").await.unwrap().is_none());
        assert!(approve(&p, &i.id).await.unwrap());
        assert!(!approve(&p, &i.id).await.unwrap(), "已不是 pending");
        assert!(claim_next(&p, "", "t").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn finish_external_becomes_draft_internal_done() {
        let p = pool().await;
        let i = insert_item(&p, new("m1", QUEUED)).await.unwrap().unwrap();
        claim_next(&p, "", "t").await.unwrap();
        assert!(finish(&p, &i.id, "答案", false).await.unwrap());
        assert_eq!(get_item(&p, &i.id).await.unwrap().unwrap().status, DRAFT);
        assert!(update_draft(&p, &i.id, "改過").await.unwrap());
        assert!(mark_sent(&p, &i.id).await.unwrap());
        assert_eq!(get_item(&p, &i.id).await.unwrap().unwrap().status, DONE);

        let j = insert_item(&p, new("m2", QUEUED)).await.unwrap().unwrap();
        claim_next(&p, "", "t").await.unwrap();
        assert!(finish(&p, &j.id, "x", true).await.unwrap());
        assert_eq!(get_item(&p, &j.id).await.unwrap().unwrap().status, DONE);
        assert!(!finish(&p, &j.id, "again", true).await.unwrap(), "只有 running 能完成");
    }

    #[tokio::test]
    async fn fail_returns_to_pending_with_reason_and_never_auto_requeues() {
        let p = pool().await;
        let i = insert_item(&p, new("m1", QUEUED)).await.unwrap().unwrap();
        claim_next(&p, "", "t").await.unwrap();
        assert!(fail(&p, &i.id, "分頁關閉").await.unwrap());
        let g = get_item(&p, &i.id).await.unwrap().unwrap();
        assert_eq!((g.status.as_str(), g.error.as_deref(), g.tab_id), (PENDING, Some("分頁關閉"), None));
        assert!(claim_next(&p, "", "t").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn recover_resets_running() {
        let p = pool().await;
        insert_item(&p, new("m1", QUEUED)).await.unwrap();
        claim_next(&p, "", "t").await.unwrap();
        assert_eq!(recover_interrupted(&p).await.unwrap(), 1);
        assert_eq!(list_items(&p, "", Some(PENDING)).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn old_database_gains_phase2_columns_and_keeps_rows() {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE intake_items (id TEXT PRIMARY KEY NOT NULL, source TEXT NOT NULL, sender TEXT NOT NULL, sender_level TEXT NOT NULL, subject TEXT NOT NULL, body TEXT NOT NULL, message_id TEXT NOT NULL UNIQUE, status TEXT NOT NULL, tab_id TEXT, reply_draft TEXT, error TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)")
            .execute(&p).await.unwrap();
        sqlx::query("INSERT INTO intake_items VALUES ('o','email','a@b.c','external','s','b','m','pending',NULL,NULL,NULL,1,1)").execute(&p).await.unwrap();
        init_schema(&p).await.unwrap();
        init_schema(&p).await.unwrap(); // 冪等
        let it = get_item(&p, "o").await.unwrap().unwrap();
        assert_eq!((it.kind, it.verified), (None, None));
        set_kind(&p, "o", "code_change", Some("abc")).await.unwrap();
        set_verification(&p, "o", "報告", true).await.unwrap();
        let it = get_item(&p, "o").await.unwrap().unwrap();
        assert_eq!((it.kind.as_deref(), it.snapshot_head.as_deref(), it.verified), (Some("code_change"), Some("abc"), Some(true)));
    }

    #[tokio::test]
    async fn senders_are_normalized_and_roundtrip() {
        let p = pool().await;
        upsert_sender(&p, "", &Sender { address: "Bob <BOB@Corp.com>".into(), level: Level::Internal, skip_auth: true }).await.unwrap();
        let l = list_senders(&p, "").await.unwrap();
        assert_eq!(l, vec![Sender { address: "bob@corp.com".into(), level: Level::Internal, skip_auth: true }]);
        assert!(delete_sender(&p, "", "BOB@corp.com").await.unwrap());
        assert!(list_senders(&p, "").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rate_count_settings_and_dropped() {
        let p = pool().await;
        insert_item(&p, new("m1", PENDING)).await.unwrap();
        insert_item(&p, new("m2", PENDING)).await.unwrap();
        assert_eq!(count_from_sender_since(&p, "", "a@corp.com", 0).await.unwrap(), 2);
        assert_eq!(count_from_sender_since(&p, "", "a@corp.com", now() + 100).await.unwrap(), 0);
        set_setting(&p, "k", "v1").await.unwrap();
        set_setting(&p, "k", "v2").await.unwrap();
        assert_eq!(get_setting(&p, "k").await.unwrap().as_deref(), Some("v2"));
        bump_dropped(&p, "", "not_allowed").await.unwrap();
        bump_dropped(&p, "", "not_allowed").await.unwrap();
        assert_eq!(dropped_counts(&p, "").await.unwrap(), vec![("not_allowed".to_string(), 2)]);
    }

    #[tokio::test]
    async fn delete_removes_the_item_and_its_attachments_but_never_a_running_one() {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        init_schema(&p).await.unwrap();
        let mk = |mid: &'static str| NewItem { project_id: "", id: None, source: "email", sender: "a@corp.com", level: Level::Internal, subject: "s", body: "b", message_id: mid, status: QUEUED };
        let a = insert_item(&p, mk("m1")).await.unwrap().unwrap();
        let b = insert_item(&p, mk("m2")).await.unwrap().unwrap();
        add_attachment(&p, &a.id, &crate::intake::attach::Saved { filename: "x.png".into(), path: "/x/x.png".into(), mime: "image/png", size: 1 }).await.unwrap();
        claim_next(&p, "", "tab").await.unwrap(); // a 變成 running
        assert!(!delete_item(&p, &a.id).await.unwrap(), "處理中不能刪");
        assert!(delete_item(&p, &b.id).await.unwrap());
        assert!(get_item(&p, &b.id).await.unwrap().is_none());
        finish(&p, &a.id, "r", false).await.unwrap();
        assert!(delete_item(&p, &a.id).await.unwrap());
        assert!(list_attachments(&p, &a.id).await.unwrap().is_empty());
        assert!(!delete_item(&p, "nope").await.unwrap());
    }

    #[tokio::test]
    async fn items_senders_and_drops_are_isolated_per_project_and_claims_only_see_their_own() {
        let p = pool().await;
        let mk = |project: &'static str, mid: &'static str| NewItem { project_id: project, id: None, source: "email", sender: "a@corp.com", level: Level::Internal, subject: "s", body: "b", message_id: mid, status: QUEUED };
        insert_item(&p, mk("a", "m1")).await.unwrap();
        insert_item(&p, mk("b", "m2")).await.unwrap();
        assert_eq!(list_items(&p, "a", None).await.unwrap().len(), 1);
        assert_eq!(claim_next(&p, "b", "tab").await.unwrap().unwrap().message_id, "m2");
        assert!(claim_next(&p, "b", "tab2").await.unwrap().is_none(), "b 沒有別的題目了，不能去領 a 的");
        assert_eq!(claim_next(&p, "a", "tab3").await.unwrap().unwrap().message_id, "m1");

        let s = Sender { address: "x@y.z".into(), level: Level::Internal, skip_auth: false };
        upsert_sender(&p, "a", &s).await.unwrap();
        assert_eq!(list_senders(&p, "a").await.unwrap().len(), 1);
        assert!(list_senders(&p, "b").await.unwrap().is_empty());
        assert!(!delete_sender(&p, "b", "x@y.z").await.unwrap(), "不能刪別的專案的白名單");

        bump_dropped(&p, "a", "not_allowed").await.unwrap();
        assert_eq!(dropped_counts(&p, "a").await.unwrap(), vec![("not_allowed".to_string(), 1)]);
        assert!(dropped_counts(&p, "b").await.unwrap().is_empty());
        assert_eq!(count_from_sender_since(&p, "a", "a@corp.com", 0).await.unwrap(), 1);
        assert_eq!(count_from_sender_since(&p, "zzz", "a@corp.com", 0).await.unwrap(), 0, "限流各專案獨立");
    }

    #[tokio::test]
    async fn legacy_data_is_adopted_by_a_project_without_touching_the_projects_own_rows() {
        let p = pool().await;
        let mk = |project: &'static str, mid: &'static str| NewItem { project_id: project, id: None, source: "email", sender: "a@corp.com", level: Level::Internal, subject: "s", body: "b", message_id: mid, status: PENDING };
        insert_item(&p, mk("", "old1")).await.unwrap();
        insert_item(&p, mk("", "old2")).await.unwrap();
        insert_item(&p, mk("other", "o1")).await.unwrap();
        let s = Sender { address: "old@corp.com".into(), level: Level::External, skip_auth: false };
        upsert_sender(&p, "", &s).await.unwrap();
        bump_dropped(&p, "", "not_allowed").await.unwrap();
        assert_eq!(project_counts(&p).await.unwrap().iter().find(|c| c.0.is_empty()).unwrap().2, 2);

        assert_eq!(adopt_legacy(&p, "proj").await.unwrap(), 2);
        assert_eq!(list_items(&p, "proj", None).await.unwrap().len(), 2);
        assert!(list_items(&p, "", None).await.unwrap().is_empty());
        assert_eq!(list_items(&p, "other", None).await.unwrap().len(), 1, "別的專案的題目不動");
        assert_eq!(list_senders(&p, "proj").await.unwrap(), vec![s]);
        assert!(list_senders(&p, "").await.unwrap().is_empty());
        assert_eq!(dropped_counts(&p, "proj").await.unwrap(), vec![("not_allowed".to_string(), 1)]);
    }

    #[tokio::test]
    async fn only_unassigned_items_can_be_assigned_to_a_project() {
        let p = pool().await;
        let mk = |project: &'static str, mid: &'static str| NewItem { project_id: project, id: None, source: "email", sender: "a@corp.com", level: Level::Internal, subject: "s", body: "b", message_id: mid, status: PENDING };
        let old = insert_item(&p, mk("", "m1")).await.unwrap().unwrap();
        let owned = insert_item(&p, mk("a", "m2")).await.unwrap().unwrap();
        assert!(assign_item(&p, &old.id, "b").await.unwrap());
        assert!(!assign_item(&p, &owned.id, "b").await.unwrap(), "已歸屬的不可被搬走");
        assert_eq!(get_item(&p, &old.id).await.unwrap().unwrap().project_id, "b");
        assert_eq!(get_item(&p, &owned.id).await.unwrap().unwrap().project_id, "a");
    }
}
