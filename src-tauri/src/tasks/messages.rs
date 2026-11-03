//! 訊息紀錄：使用者對執行中卡片的「補充一句」、以及 Agent 之間的 `tell_agent`。
//! 這些話直接打進對方的對話，事後沒有別處看得到；留一筆，活動記錄才查得到
//! 「誰在什麼時候對誰說過什麼」。

use serde::Serialize;
use sqlx::SqlitePool;

pub const KIND_NUDGE: &str = "nudge";
pub const KIND_TELL: &str = "tell";
/// 存下來的字數上限（來源已限 2000／500 字，這裡再保險，免得紀錄表無限長大）。
const KEEP_CHARS: usize = 600;

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct MessageRow {
    pub id: i64,
    /// Unix 秒。
    pub at: i64,
    pub kind: String,
    /// 收到這句話的卡片。
    pub task_id: String,
    /// 說話的 Agent；使用者補充是 `None`。
    pub from_agent: Option<String>,
    /// 收到的 Agent。
    pub to_agent: Option<String>,
    pub text: String,
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS card_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at INTEGER NOT NULL,
            kind TEXT NOT NULL,
            task_id TEXT NOT NULL,
            from_agent TEXT,
            to_agent TEXT,
            text TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record(
    pool: &SqlitePool,
    kind: &str,
    task_id: &str,
    from_agent: Option<&str>,
    to_agent: Option<&str>,
    text: &str,
) -> Result<(), sqlx::Error> {
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    record_at(pool, at, kind, task_id, from_agent, to_agent, text).await
}

pub async fn record_at(
    pool: &SqlitePool,
    at: i64,
    kind: &str,
    task_id: &str,
    from_agent: Option<&str>,
    to_agent: Option<&str>,
    text: &str,
) -> Result<(), sqlx::Error> {
    let text: String = text.chars().take(KEEP_CHARS).collect();
    sqlx::query("INSERT INTO card_messages (at, kind, task_id, from_agent, to_agent, text) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(at)
        .bind(kind)
        .bind(task_id)
        .bind(from_agent)
        .bind(to_agent)
        .bind(text)
        .execute(pool)
        .await?;
    Ok(())
}

/// 最近的在前。
pub async fn recent(pool: &SqlitePool, limit: i64) -> Result<Vec<MessageRow>, sqlx::Error> {
    sqlx::query_as::<_, MessageRow>("SELECT * FROM card_messages ORDER BY at DESC, id DESC LIMIT ?").bind(limit).fetch_all(pool).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn messages_come_back_newest_first_with_who_said_what_to_whom() {
        let pool = pool().await;
        record_at(&pool, 100, KIND_NUDGE, "t1", None, Some("a1"), "順便改 X").await.unwrap();
        record_at(&pool, 200, KIND_TELL, "t2", Some("a1"), Some("a2"), "我改名了").await.unwrap();
        let rows = recent(&pool, 10).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["我改名了", "順便改 X"]);
        assert_eq!((rows[0].kind.as_str(), rows[0].from_agent.as_deref(), rows[0].to_agent.as_deref()), (KIND_TELL, Some("a1"), Some("a2")));
        assert_eq!((rows[1].kind.as_str(), rows[1].from_agent.as_deref(), rows[1].task_id.as_str()), (KIND_NUDGE, None, "t1"));
    }

    #[tokio::test]
    async fn the_limit_keeps_only_the_newest() {
        let pool = pool().await;
        for i in 0..5 {
            record_at(&pool, i, KIND_NUDGE, "t", None, None, &format!("m{i}")).await.unwrap();
        }
        let rows = recent(&pool, 2).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["m4", "m3"]);
    }

    #[tokio::test]
    async fn an_oversized_text_is_cut_so_the_table_cannot_grow_without_bound() {
        let pool = pool().await;
        record(&pool, KIND_NUDGE, "t", None, None, &"字".repeat(KEEP_CHARS + 50)).await.unwrap();
        assert_eq!(recent(&pool, 1).await.unwrap()[0].text.chars().count(), KEEP_CHARS);
    }
}
