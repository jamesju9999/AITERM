//! LINE 群組的逐則訊息與「還沒加入白名單的來源」。純資料層，不碰網路與 Tauri。
//! 設計：docs/superpowers/specs/2026-11-23-intake-line-groups-design.md
use serde::Serialize;
use sqlx::SqlitePool;

pub async fn init(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    for q in [
        "CREATE TABLE IF NOT EXISTS line_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chat TEXT NOT NULL,
            msg_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            name TEXT NOT NULL,
            text TEXT NOT NULL,
            ts INTEGER NOT NULL,
            outgoing INTEGER NOT NULL DEFAULT 0,
            handled INTEGER NOT NULL DEFAULT 0,
            UNIQUE (chat, msg_id)
        )",
        "CREATE INDEX IF NOT EXISTS idx_line_messages_chat ON line_messages(chat, id)",
        "CREATE INDEX IF NOT EXISTS idx_line_messages_ts ON line_messages(ts)",
        "CREATE TABLE IF NOT EXISTS line_names (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS line_unknown (
            id TEXT PRIMARY KEY NOT NULL,
            kind TEXT NOT NULL,
            name TEXT NOT NULL DEFAULT '',
            count INTEGER NOT NULL DEFAULT 0,
            first_seen INTEGER NOT NULL,
            last_seen INTEGER NOT NULL
        )",
    ] {
        sqlx::query(q).execute(pool).await?;
    }
    Ok(())
}

pub struct NewMessage<'a> {
    pub chat: &'a str,
    pub msg_id: &'a str,
    pub user_id: &'a str,
    pub name: &'a str,
    pub text: &'a str,
    pub ts: i64,
    pub outgoing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct LineMessage {
    pub id: i64,
    pub chat: String,
    pub msg_id: String,
    pub user_id: String,
    pub name: String,
    pub text: String,
    pub ts: i64,
    pub outgoing: i64,
    pub handled: i64,
}

/// 存一則訊息；同一群組同一則 id 只存一次（LINE 重送時回 `None`）。成功回傳資料列 id。
pub async fn insert_message(pool: &SqlitePool, m: &NewMessage<'_>) -> Result<Option<i64>, sqlx::Error> {
    let r = sqlx::query("INSERT OR IGNORE INTO line_messages (chat, msg_id, user_id, name, text, ts, outgoing) VALUES (?,?,?,?,?,?,?)")
        .bind(m.chat)
        .bind(m.msg_id)
        .bind(m.user_id)
        .bind(m.name)
        .bind(m.text)
        .bind(m.ts)
        .bind(m.outgoing as i64)
        .execute(pool)
        .await?;
    Ok((r.rows_affected() > 0).then(|| r.last_insert_rowid()))
}

pub async fn get_by_msg_id(pool: &SqlitePool, chat: &str, msg_id: &str) -> Result<Option<LineMessage>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM line_messages WHERE chat = ? AND msg_id = ?").bind(chat).bind(msg_id).fetch_optional(pool).await
}

/// 某則（資料列 id）之前、`since_ts` 之後的最近 `limit` 則，舊的在前。
pub async fn context_before(pool: &SqlitePool, chat: &str, before_id: i64, since_ts: i64, limit: i64) -> Result<Vec<LineMessage>, sqlx::Error> {
    let mut v: Vec<LineMessage> = sqlx::query_as("SELECT * FROM line_messages WHERE chat = ? AND id < ? AND ts >= ? ORDER BY id DESC LIMIT ?")
        .bind(chat)
        .bind(before_id)
        .bind(since_ts)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    v.reverse();
    Ok(v)
}

/// 還沒處理過的、不是我方（AITerm 送出）的訊息，舊的在前。
pub async fn unhandled(pool: &SqlitePool, chat: &str) -> Result<Vec<LineMessage>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM line_messages WHERE chat = ? AND handled = 0 AND outgoing = 0 ORDER BY id").bind(chat).fetch_all(pool).await
}

pub async fn chats_with_unhandled(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT DISTINCT chat FROM line_messages WHERE handled = 0 AND outgoing = 0").fetch_all(pool).await
}

/// 某則（資料列 id）之後的所有訊息，舊的在前。
pub async fn after(pool: &SqlitePool, chat: &str, after_id: i64) -> Result<Vec<LineMessage>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM line_messages WHERE chat = ? AND id > ? ORDER BY id").bind(chat).bind(after_id).fetch_all(pool).await
}

/// 把這個群組資料列 id ≤ `upto_id` 的訊息都標成已處理。
pub async fn mark_handled(pool: &SqlitePool, chat: &str, upto_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE line_messages SET handled = 1 WHERE chat = ? AND id <= ?").bind(chat).bind(upto_id).execute(pool).await?;
    Ok(())
}

/// 刪掉 `older_than_ts` 之前的訊息，回傳刪了幾則。
pub async fn purge(pool: &SqlitePool, older_than_ts: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM line_messages WHERE ts < ?").bind(older_than_ts).execute(pool).await?.rows_affected())
}

/// 依保存天數清掉過期的訊息；0 天＝不保存，全部清掉。
pub async fn purge_expired(pool: &SqlitePool, retention_days: u32, now: i64) -> Result<u64, sqlx::Error> {
    let cutoff = if retention_days == 0 { i64::MAX } else { now - i64::from(retention_days) * 86_400 };
    purge(pool, cutoff).await
}

pub async fn message_count(pool: &SqlitePool, chat: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM line_messages WHERE chat = ?").bind(chat).fetch_one(pool).await
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Unknown {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub count: i64,
    pub first_seen: i64,
    pub last_seen: i64,
}

/// 記下一個還沒加入白名單的來源（只記編號、名稱、次數，不存訊息內容）。`name` 空白時保留舊的。
pub async fn note_unknown(pool: &SqlitePool, id: &str, kind: &str, name: &str, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO line_unknown (id, kind, name, count, first_seen, last_seen) VALUES (?,?,?,1,?,?)
         ON CONFLICT(id) DO UPDATE SET count = count + 1, last_seen = excluded.last_seen,
                                       name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE name END",
    )
    .bind(id)
    .bind(kind)
    .bind(name)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// 使用者或群組的顯示名稱快取（編號一律小寫）。
pub async fn set_name(pool: &SqlitePool, id: &str, name: &str) -> Result<(), sqlx::Error> {
    let name = name.chars().filter(|c| !c.is_control()).collect::<String>();
    let name = name.trim();
    if name.is_empty() {
        return Ok(());
    }
    let name: String = name.chars().take(80).collect();
    sqlx::query("INSERT INTO line_names (id, name) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name").bind(id.to_lowercase()).bind(name).execute(pool).await?;
    Ok(())
}

pub async fn get_name(pool: &SqlitePool, id: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM line_names WHERE id = ?").bind(id.to_lowercase()).fetch_optional(pool).await
}

pub async fn list_unknown(pool: &SqlitePool) -> Result<Vec<Unknown>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM line_unknown ORDER BY last_seen DESC, id").fetch_all(pool).await
}

pub async fn unknown_name(pool: &SqlitePool, id: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM line_unknown WHERE id = ?").bind(id).fetch_optional(pool).await
}

pub async fn forget_unknown(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM line_unknown WHERE id = ?").bind(id).execute(pool).await?.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        init(&p).await.unwrap();
        p
    }
    fn msg<'a>(chat: &'a str, id: &'a str, ts: i64, text: &'a str) -> NewMessage<'a> {
        NewMessage { chat, msg_id: id, user_id: "u1", name: "介琳", text, ts, outgoing: false }
    }

    #[tokio::test]
    async fn a_message_is_stored_once_per_chat_and_redelivery_is_ignored() {
        let p = pool().await;
        assert!(insert_message(&p, &msg("c1", "m1", 100, "hi")).await.unwrap().is_some());
        assert!(insert_message(&p, &msg("c1", "m1", 100, "hi")).await.unwrap().is_none());
        assert!(insert_message(&p, &msg("c2", "m1", 100, "同一個 id 在別的群組")).await.unwrap().is_some());
        assert_eq!(message_count(&p, "c1").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn context_is_the_latest_messages_before_a_row_in_order_within_the_time_window() {
        let p = pool().await;
        let mut ids = vec![];
        for (i, ts) in [10, 20, 30, 40, 50].into_iter().enumerate() {
            ids.push(insert_message(&p, &msg("c1", &format!("m{i}"), ts, &format!("t{i}"))).await.unwrap().unwrap());
        }
        insert_message(&p, &msg("c2", "x", 45, "別的群組")).await.unwrap();
        // 第 5 則之前，只要 ts>=20 的最近 2 則：m2、m3。
        let c = context_before(&p, "c1", ids[4], 20, 2).await.unwrap();
        assert_eq!(c.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["t2", "t3"]);
        // 時間窗外的不取、本身與之後的不取。
        let c = context_before(&p, "c1", ids[4], 35, 30).await.unwrap();
        assert_eq!(c.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["t3"]);
        assert!(context_before(&p, "c1", ids[0], 0, 30).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unhandled_excludes_our_own_messages_and_handled_ones_and_marks_up_to_a_row() {
        let p = pool().await;
        let a = insert_message(&p, &msg("c1", "a", 1, "客戶 A")).await.unwrap().unwrap();
        insert_message(&p, &NewMessage { outgoing: true, user_id: "", name: "AI", ..msg("c1", "ai", 2, "我們的回覆") }).await.unwrap();
        let b = insert_message(&p, &msg("c1", "b", 3, "客戶 B")).await.unwrap().unwrap();
        assert_eq!(unhandled(&p, "c1").await.unwrap().iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["客戶 A", "客戶 B"]);
        assert_eq!(chats_with_unhandled(&p).await.unwrap(), vec!["c1".to_string()]);
        mark_handled(&p, "c1", a).await.unwrap();
        assert_eq!(unhandled(&p, "c1").await.unwrap().len(), 1);
        mark_handled(&p, "c1", b).await.unwrap();
        assert!(unhandled(&p, "c1").await.unwrap().is_empty());
        assert!(chats_with_unhandled(&p).await.unwrap().is_empty());
        // 之後的訊息（含我方）可查。
        assert_eq!(after(&p, "c1", a).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn names_are_cached_case_insensitively_cleaned_and_blank_ones_ignored() {
        let p = pool().await;
        set_name(&p, "Uabc", "  介琳\n").await.unwrap();
        set_name(&p, "uabc", "").await.unwrap();
        assert_eq!(get_name(&p, "UABC").await.unwrap().as_deref(), Some("介琳"));
        assert_eq!(get_name(&p, "nobody").await.unwrap(), None);
    }

    #[tokio::test]
    async fn purge_deletes_only_messages_older_than_the_cutoff() {
        let p = pool().await;
        insert_message(&p, &msg("c1", "old", 100, "舊")).await.unwrap();
        insert_message(&p, &msg("c1", "new", 200, "新")).await.unwrap();
        assert_eq!(purge(&p, 150).await.unwrap(), 1);
        assert!(get_by_msg_id(&p, "c1", "old").await.unwrap().is_none());
        assert!(get_by_msg_id(&p, "c1", "new").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn unknown_sources_are_counted_keep_the_last_known_name_and_can_be_forgotten() {
        let p = pool().await;
        note_unknown(&p, "c1", "group", "", 10).await.unwrap();
        note_unknown(&p, "c1", "group", "兆強科技客服群", 20).await.unwrap();
        note_unknown(&p, "c1", "group", "", 30).await.unwrap();
        note_unknown(&p, "u1", "user", "介琳", 25).await.unwrap();
        let l = list_unknown(&p).await.unwrap();
        assert_eq!(l.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), vec!["c1", "u1"], "最近出現的在前");
        assert_eq!((l[0].count, l[0].name.as_str(), l[0].first_seen, l[0].last_seen), (3, "兆強科技客服群", 10, 30));
        assert_eq!(unknown_name(&p, "u1").await.unwrap().as_deref(), Some("介琳"));
        assert!(forget_unknown(&p, "c1").await.unwrap());
        assert!(!forget_unknown(&p, "c1").await.unwrap());
        assert_eq!(list_unknown(&p).await.unwrap().len(), 1);
    }
}
