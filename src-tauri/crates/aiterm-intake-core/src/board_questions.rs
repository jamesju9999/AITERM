//! Agent 中途問人（`ask_human`）：server 上的 Agent 提問後掛起，等任何成員（或 Telegram 上的主人）回答。
//! 見 docs/superpowers/specs/2026-11-27-server-ask-human-formations-design.md。
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

pub const OPEN: &str = "open";
pub const ANSWERED: &str = "answered";
pub const CANCELLED: &str = "cancelled";

pub const MAX_OPTIONS: usize = 6;
pub const MAX_QUESTION_CHARS: usize = 2000;
/// 沒人回答時：距上次提醒（或提問）多久再提醒、最多提醒幾次。
pub const REMIND_EVERY_SECS: i64 = 30 * 60;
pub const MAX_REMINDERS: i64 = 3;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct BoardQuestion {
    pub id: i64,
    pub project_id: String,
    pub card_id: String,
    pub agent_id: Option<String>,
    pub question: String,
    /// 選項的 JSON 陣列字串。
    pub options: String,
    pub recommended: Option<String>,
    pub context: Option<String>,
    pub status: String,
    pub answer: Option<String>,
    pub answered_by: Option<String>,
    /// `app` | `telegram`
    pub answered_via: Option<String>,
    pub telegram_message_id: Option<i64>,
    pub reminders: i64,
    pub last_reminded_at: Option<i64>,
    pub created_at: i64,
    pub answered_at: Option<i64>,
}

impl BoardQuestion {
    pub fn option_list(&self) -> Vec<String> {
        serde_json::from_str(&self.options).unwrap_or_default()
    }
    pub fn label(&self) -> String {
        format!("Q-{}", self.id)
    }
}

pub struct NewQuestion<'a> {
    pub project_id: &'a str,
    pub card_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub question: &'a str,
    pub options: &'a [String],
    pub recommended: Option<&'a str>,
    pub context: Option<&'a str>,
}

/// 新增一個問題。同一張卡同時只能有一個未答的；問題空白、太長、選項太多都拒絕。
pub async fn ask(pool: &SqlitePool, q: &NewQuestion<'_>) -> Result<Result<i64, String>, sqlx::Error> {
    let question = q.question.trim();
    if question.is_empty() {
        return Ok(Err("問題不能是空的".into()));
    }
    if question.chars().count() > MAX_QUESTION_CHARS {
        return Ok(Err(format!("問題太長（上限 {MAX_QUESTION_CHARS} 字）")));
    }
    let options: Vec<String> = q.options.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
    if options.len() > MAX_OPTIONS {
        return Ok(Err(format!("選項最多 {MAX_OPTIONS} 個")));
    }
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_questions WHERE card_id = ? AND status = 'open'").bind(q.card_id).fetch_one(pool).await?;
    if open > 0 {
        return Ok(Err("這張卡已經有一個問題在等回答，等它被回答再問下一個。".into()));
    }
    let id = sqlx::query("INSERT INTO board_questions (project_id, card_id, agent_id, question, options, recommended, context, created_at) VALUES (?,?,?,?,?,?,?,?)")
        .bind(q.project_id).bind(q.card_id).bind(q.agent_id).bind(question)
        .bind(serde_json::to_string(&options).unwrap_or_else(|_| "[]".into()))
        .bind(q.recommended).bind(q.context).bind(now())
        .execute(pool).await?.last_insert_rowid();
    Ok(Ok(id))
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<BoardQuestion>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_questions WHERE id = ?").bind(id).fetch_optional(pool).await
}

/// 回答。只有還在 `open` 的能回答；已答／已取消（或不存在）回 `false`，不會被改寫。
pub async fn answer(pool: &SqlitePool, id: i64, text: &str, by: &str, via: &str) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE board_questions SET status = 'answered', answer = ?, answered_by = ?, answered_via = ?, answered_at = ? WHERE id = ? AND status = 'open'")
        .bind(text).bind(by).bind(via).bind(now()).bind(id).execute(pool).await?;
    Ok(r.rows_affected() > 0)
}

pub async fn cancel(pool: &SqlitePool, id: i64, by: &str) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE board_questions SET status = 'cancelled', answered_by = ?, answered_at = ? WHERE id = ? AND status = 'open'")
        .bind(by).bind(now()).bind(id).execute(pool).await?;
    Ok(r.rows_affected() > 0)
}

pub async fn open_questions(pool: &SqlitePool, project_id: &str) -> Result<Vec<BoardQuestion>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_questions WHERE project_id = ? AND status = 'open' ORDER BY id").bind(project_id).fetch_all(pool).await
}

pub async fn recent_closed(pool: &SqlitePool, project_id: &str, limit: i64) -> Result<Vec<BoardQuestion>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_questions WHERE project_id = ? AND status != 'open' ORDER BY id DESC LIMIT ?").bind(project_id).bind(limit.clamp(1, 200)).fetch_all(pool).await
}

/// 全部專案還沒回答的問題（提醒用）。
pub async fn all_open(pool: &SqlitePool) -> Result<Vec<BoardQuestion>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_questions WHERE status = 'open' ORDER BY id").fetch_all(pool).await
}

pub async fn set_telegram_message(pool: &SqlitePool, id: i64, message_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_questions SET telegram_message_id = ? WHERE id = ?").bind(message_id).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn by_telegram_message(pool: &SqlitePool, message_id: i64) -> Result<Option<BoardQuestion>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM board_questions WHERE telegram_message_id = ?").bind(message_id).fetch_optional(pool).await
}

pub async fn mark_reminded(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_questions SET reminders = reminders + 1, last_reminded_at = ? WHERE id = ?").bind(now()).bind(id).execute(pool).await?;
    Ok(())
}

/// 這張卡結束了：還在等的問題沒有人在等答案了，標成取消。
pub async fn close_card(pool: &SqlitePool, card_id: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE board_questions SET status = 'cancelled', answered_by = 'system', answered_at = ? WHERE card_id = ? AND status = 'open'").bind(now()).bind(card_id).execute(pool).await?.rows_affected())
}

/// server 重啟：所有還在等的問題都已經沒有行程在等了。
pub async fn close_dangling(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE board_questions SET status = 'cancelled', answered_by = 'system', answered_at = ? WHERE status = 'open'").bind(now()).execute(pool).await?.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        crate::board_store::init_schema(&p).await.unwrap();
        p
    }

    fn q<'a>(card: &'a str, text: &'a str, opts: &'a [String]) -> NewQuestion<'a> {
        NewQuestion { project_id: "P", card_id: card, agent_id: Some("A"), question: text, options: opts, recommended: None, context: None }
    }

    #[tokio::test]
    async fn a_card_can_only_have_one_open_question_until_it_is_answered() {
        let p = pool().await;
        let id = ask(&p, &q("c1", "用 A 還是 B？", &[])).await.unwrap().unwrap();
        assert!(ask(&p, &q("c1", "再問一個", &[])).await.unwrap().is_err(), "同卡不能同時有兩個未答問題");
        assert!(ask(&p, &q("c2", "別張卡可以", &[])).await.unwrap().is_ok());
        assert!(answer(&p, id, "A", "小明", "app").await.unwrap());
        assert!(ask(&p, &q("c1", "答完就能再問", &[])).await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn only_the_first_answer_counts_and_cancelled_cannot_be_answered() {
        let p = pool().await;
        let id = ask(&p, &q("c1", "問", &[])).await.unwrap().unwrap();
        assert!(answer(&p, id, "第一個", "甲", "app").await.unwrap());
        assert!(!answer(&p, id, "第二個", "乙", "telegram").await.unwrap(), "已答的不能被改寫");
        assert!(!cancel(&p, id, "admin").await.unwrap());
        let row = get(&p, id).await.unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.answer.as_deref(), row.answered_by.as_deref(), row.answered_via.as_deref()), (ANSWERED, Some("第一個"), Some("甲"), Some("app")));
        let id2 = ask(&p, &q("c2", "問", &[])).await.unwrap().unwrap();
        assert!(cancel(&p, id2, "admin").await.unwrap());
        assert!(!answer(&p, id2, "x", "甲", "app").await.unwrap());
    }

    #[tokio::test]
    async fn invalid_questions_are_rejected_and_options_are_cleaned() {
        let p = pool().await;
        assert!(ask(&p, &q("c", "   ", &[])).await.unwrap().is_err());
        assert!(ask(&p, &q("c", &"長".repeat(MAX_QUESTION_CHARS + 1), &[])).await.unwrap().is_err());
        let many: Vec<String> = (0..7).map(|i| i.to_string()).collect();
        assert!(ask(&p, &q("c", "問", &many)).await.unwrap().is_err());
        let id = ask(&p, &q("c", "問", &[" 甲 ".into(), "".into(), "乙".into()])).await.unwrap().unwrap();
        assert_eq!(get(&p, id).await.unwrap().unwrap().option_list(), vec!["甲", "乙"]);
    }

    #[tokio::test]
    async fn closing_a_card_or_restarting_cancels_only_open_questions() {
        let p = pool().await;
        let a = ask(&p, &q("c1", "問", &[])).await.unwrap().unwrap();
        let b = ask(&p, &q("c2", "問", &[])).await.unwrap().unwrap();
        let c = ask(&p, &q("c3", "問", &[])).await.unwrap().unwrap();
        answer(&p, c, "好", "甲", "app").await.unwrap();
        assert_eq!(close_card(&p, "c1").await.unwrap(), 1);
        assert_eq!(get(&p, a).await.unwrap().unwrap().status, CANCELLED);
        assert_eq!(close_dangling(&p).await.unwrap(), 1);
        assert_eq!(get(&p, b).await.unwrap().unwrap().status, CANCELLED);
        assert_eq!(get(&p, c).await.unwrap().unwrap().status, ANSWERED, "已答的不受影響");
        assert!(open_questions(&p, "P").await.unwrap().is_empty());
        assert_eq!(recent_closed(&p, "P", 10).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn telegram_message_lookup_and_reminders() {
        let p = pool().await;
        let id = ask(&p, &q("c1", "問", &[])).await.unwrap().unwrap();
        set_telegram_message(&p, id, 777).await.unwrap();
        assert_eq!(by_telegram_message(&p, 777).await.unwrap().unwrap().id, id);
        mark_reminded(&p, id).await.unwrap();
        let row = get(&p, id).await.unwrap().unwrap();
        assert_eq!(row.reminders, 1);
        assert!(row.last_reminded_at.is_some());
        assert_eq!(all_open(&p).await.unwrap().len(), 1);
    }
}
