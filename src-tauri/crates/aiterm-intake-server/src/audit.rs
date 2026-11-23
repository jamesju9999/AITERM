//! 稽核紀錄：誰、何時、對哪一題、做了什麼。
use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Serialize, Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Entry {
    pub ts: i64,
    pub member: String,
    pub action: String,
    pub target: String,
}

pub async fn record(pool: &SqlitePool, member: &str, action: &str, target: &str) {
    let r = sqlx::query("INSERT INTO audit(ts, member, action, target) VALUES (?,?,?,?)")
        .bind(crate::db::now())
        .bind(member)
        .bind(action)
        .bind(target)
        .execute(pool)
        .await;
    if let Err(e) = r {
        log::warn!("audit: 寫入失敗：{e}");
    }
}

pub async fn recent(pool: &SqlitePool, limit: i64) -> Vec<Entry> {
    sqlx::query_as("SELECT ts, member, action, target FROM audit ORDER BY id DESC LIMIT ?").bind(limit).fetch_all(pool).await.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn entries_are_recorded_newest_first() {
        let p = crate::db::open_memory().await.unwrap().server;
        record(&p, "小明", "approve", "item-1").await;
        record(&p, "小華", "reply", "item-2").await;
        let r = recent(&p, 10).await;
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].member.as_str(), r[0].action.as_str(), r[0].target.as_str()), ("小華", "reply", "item-2"));
    }
}
