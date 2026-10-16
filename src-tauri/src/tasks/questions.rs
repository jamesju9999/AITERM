//! Agent 需要人決定時提出的問題（`ask_human`）。存在專案的 tasks.db，
//! App 收件匣與 Telegram 都從這裡讀寫。見
//! docs/superpowers/specs/2026-10-16-agent-ask-human-design.md。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

pub const OPEN: &str = "open";
pub const ANSWERED: &str = "answered";
pub const CANCELLED: &str = "cancelled";

/// 沒人回答時：距上次提醒（或提問）多久再提醒一次、最多提醒幾次。
pub const REMIND_EVERY_SECS: i64 = 30 * 60;
pub const MAX_REMINDERS: i64 = 3;
/// 選項最多幾個（Telegram 一列一個按鈕，太多不好按）。
pub const MAX_OPTIONS: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct QuestionRow {
    pub id: i64,
    pub task_id: String,
    pub agent_id: Option<String>,
    pub question: String,
    /// 選項的 JSON 陣列字串。
    pub options: String,
    pub recommended: Option<String>,
    pub context: Option<String>,
    pub status: String,
    pub answer: Option<String>,
    /// `app` | `telegram`
    pub answered_via: Option<String>,
    pub telegram_message_id: Option<i64>,
    pub reminders: i64,
    pub last_reminded_at: Option<i64>,
    pub created_at: i64,
    pub answered_at: Option<i64>,
}

impl QuestionRow {
    pub fn option_list(&self) -> Vec<String> {
        serde_json::from_str(&self.options).unwrap_or_default()
    }

    /// 畫面與訊息上的編號。
    pub fn label(&self) -> String {
        format!("Q-{}", self.id)
    }
}

/// `ask_human` 的參數。
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct NewQuestion {
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub recommended: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS questions (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id             TEXT NOT NULL,
            agent_id            TEXT,
            question            TEXT NOT NULL,
            options             TEXT NOT NULL DEFAULT '[]',
            recommended         TEXT,
            context             TEXT,
            status              TEXT NOT NULL DEFAULT 'open',
            answer              TEXT,
            answered_via        TEXT,
            telegram_message_id INTEGER,
            reminders           INTEGER NOT NULL DEFAULT 0,
            last_reminded_at    INTEGER,
            created_at          INTEGER NOT NULL,
            answered_at         INTEGER
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_questions_open ON questions(status, task_id)")
        .execute(pool)
        .await?;
    Ok(())
}

/// 建立問題。空白的選項丟掉、最多留 `MAX_OPTIONS` 個；建議選項不在選項裡就不記。
pub async fn create(pool: &SqlitePool, task_id: &str, agent_id: Option<&str>, q: &NewQuestion) -> Result<QuestionRow, sqlx::Error> {
    let question = q.question.trim();
    if question.is_empty() {
        return Err(sqlx::Error::Protocol("question 不能是空的".into()));
    }
    let options: Vec<String> = q
        .options
        .iter()
        .map(|o| o.trim().to_string())
        .filter(|o| !o.is_empty())
        .take(MAX_OPTIONS)
        .collect();
    let recommended = q.recommended.as_deref().map(str::trim).filter(|r| options.iter().any(|o| o == r));
    let context = q.context.as_deref().map(str::trim).filter(|c| !c.is_empty());
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO questions (task_id, agent_id, question, options, recommended, context, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(task_id)
    .bind(agent_id)
    .bind(question)
    .bind(serde_json::to_string(&options).unwrap_or_else(|_| "[]".into()))
    .bind(recommended)
    .bind(context)
    .bind(crate::tasks::store::now_secs())
    .fetch_one(pool)
    .await?;
    get(pool, id).await?.ok_or(sqlx::Error::RowNotFound)
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<QuestionRow>, sqlx::Error> {
    sqlx::query_as::<_, QuestionRow>("SELECT * FROM questions WHERE id = ?").bind(id).fetch_optional(pool).await
}

/// 未回答、而且提問的卡片還在執行中的問題（新的在前）。卡片已經結束的不列。
pub async fn list_open(pool: &SqlitePool) -> Result<Vec<QuestionRow>, sqlx::Error> {
    sqlx::query_as::<_, QuestionRow>(
        "SELECT q.* FROM questions q JOIN tasks t ON t.id = q.task_id
         WHERE q.status = 'open' AND t.status = 'running' ORDER BY q.id DESC",
    )
    .fetch_all(pool)
    .await
}

/// 回答一題。只接受還沒回答的；回傳更新後的那一列，已經答過或取消的回 `None`。
pub async fn answer(pool: &SqlitePool, id: i64, answer: &str, via: &str) -> Result<Option<QuestionRow>, sqlx::Error> {
    let answer = answer.trim();
    if answer.is_empty() {
        return Err(sqlx::Error::Protocol("回答不能是空的".into()));
    }
    let n = sqlx::query(
        "UPDATE questions SET status = 'answered', answer = ?, answered_via = ?, answered_at = ?
         WHERE id = ? AND status = 'open'",
    )
    .bind(answer)
    .bind(via)
    .bind(crate::tasks::store::now_secs())
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    if n == 0 {
        return Ok(None);
    }
    get(pool, id).await
}

/// 卡片結束（完成、失敗、停止）：它還沒回答的問題作廢。回傳作廢的那些。
pub async fn cancel_open_for_task(pool: &SqlitePool, task_id: &str) -> Result<Vec<QuestionRow>, sqlx::Error> {
    let open: Vec<QuestionRow> =
        sqlx::query_as("SELECT * FROM questions WHERE task_id = ? AND status = 'open'").bind(task_id).fetch_all(pool).await?;
    sqlx::query("UPDATE questions SET status = 'cancelled' WHERE task_id = ? AND status = 'open'")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(open)
}

pub async fn has_open(pool: &SqlitePool, task_id: &str) -> Result<bool, sqlx::Error> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM questions WHERE task_id = ? AND status = 'open'")
        .bind(task_id)
        .fetch_one(pool)
        .await?;
    Ok(n > 0)
}

pub async fn set_telegram_message(pool: &SqlitePool, id: i64, message_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE questions SET telegram_message_id = ? WHERE id = ?").bind(message_id).bind(id).execute(pool).await?;
    Ok(())
}

/// 用 Telegram 訊息 id 找問題（使用者「回覆」那則問題訊息時）。
pub async fn by_telegram_message(pool: &SqlitePool, message_id: i64) -> Result<Option<QuestionRow>, sqlx::Error> {
    sqlx::query_as::<_, QuestionRow>("SELECT * FROM questions WHERE telegram_message_id = ? ORDER BY id DESC LIMIT 1")
        .bind(message_id)
        .fetch_optional(pool)
        .await
}

/// 該提醒的問題：未回答、卡片還在跑、距上次提醒（沒提醒過就是提問時間）滿 `REMIND_EVERY_SECS`、
/// 提醒次數未滿。
pub async fn due_reminders(pool: &SqlitePool, now: i64) -> Result<Vec<QuestionRow>, sqlx::Error> {
    Ok(list_open(pool)
        .await?
        .into_iter()
        .filter(|q| q.reminders < MAX_REMINDERS && now - q.last_reminded_at.unwrap_or(q.created_at) >= REMIND_EVERY_SECS)
        .collect())
}

pub async fn mark_reminded(pool: &SqlitePool, id: i64, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE questions SET reminders = reminders + 1, last_reminded_at = ? WHERE id = ?")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// `ask_human` 立刻回給 Claude 的文字：請它結束這一輪等答案。
pub fn ask_reply(q: &QuestionRow) -> String {
    format!(
        "已送出問題 {}。請現在結束這一輪回覆，等待答案；答案會以新訊息送來。\
         在答案來之前不要自己做這個決定，也不要繼續做依賴這個決定的工作，也不要印出完成標記。",
        q.label()
    )
}

/// 回答送回分頁的文字。最後再提醒一次完成標記（問題未答時它不會印，答完要記得）。
pub fn answer_message(q: &QuestionRow, marker_instruction: &str) -> String {
    format!(
        "{} 的回答：{}\n（請依這個答案繼續。全部做完之後：{}）",
        q.label(),
        q.answer.as_deref().unwrap_or(""),
        marker_instruction
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool_with_running_task() -> (SqlitePool, String) {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        let id = crate::tasks::store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        crate::tasks::store::move_task(&pool, &id, crate::tasks::store::STATUS_QUEUED, 1.0).await.unwrap();
        crate::tasks::store::dispatch_for_test(&pool, &id, "tab").await;
        (pool, id)
    }

    fn q(question: &str, options: &[&str], recommended: Option<&str>) -> NewQuestion {
        NewQuestion {
            question: question.into(),
            options: options.iter().map(|s| s.to_string()).collect(),
            recommended: recommended.map(str::to_string),
            context: Some("  背景  ".into()),
        }
    }

    #[tokio::test]
    async fn create_cleans_options_and_keeps_a_valid_recommendation() {
        let (pool, task) = pool_with_running_task().await;
        let row = create(&pool, &task, Some("a1"), &q(" 名稱用中文？ ", &["中文", "  ", "英文"], Some("英文"))).await.unwrap();
        assert_eq!(row.question, "名稱用中文？");
        assert_eq!(row.option_list(), vec!["中文", "英文"]);
        assert_eq!(row.recommended.as_deref(), Some("英文"));
        assert_eq!(row.context.as_deref(), Some("背景"));
        assert_eq!((row.status.as_str(), row.label()), (OPEN, format!("Q-{}", row.id)));
        // 建議的不是任何一個選項：不記。選項太多：只留前面幾個。
        let many: Vec<String> = (0..10).map(|i| format!("選項{i}")).collect();
        let row = create(&pool, &task, None, &NewQuestion { question: "x".into(), options: many, recommended: Some("沒有這個".into()), context: None })
            .await
            .unwrap();
        assert_eq!(row.option_list().len(), MAX_OPTIONS);
        assert_eq!(row.recommended, None);
        assert!(create(&pool, &task, None, &q("   ", &[], None)).await.is_err());
    }

    #[tokio::test]
    async fn answering_works_once_and_only_open_running_questions_are_listed() {
        let (pool, task) = pool_with_running_task().await;
        let a = create(&pool, &task, None, &q("A？", &["是", "否"], None)).await.unwrap();
        let b = create(&pool, &task, None, &q("B？", &[], None)).await.unwrap();
        assert_eq!(list_open(&pool).await.unwrap().iter().map(|x| x.id).collect::<Vec<_>>(), vec![b.id, a.id]);
        assert!(has_open(&pool, &task).await.unwrap());

        let done = answer(&pool, a.id, " 是 ", "telegram").await.unwrap().unwrap();
        assert_eq!((done.status.as_str(), done.answer.as_deref(), done.answered_via.as_deref()), (ANSWERED, Some("是"), Some("telegram")));
        assert!(answer(&pool, a.id, "否", "app").await.unwrap().is_none(), "答過就不能再答");
        assert!(answer(&pool, b.id, "  ", "app").await.is_err());

        // 卡片結束：還沒答的作廢，也不再列出。
        let cancelled = cancel_open_for_task(&pool, &task).await.unwrap();
        assert_eq!(cancelled.iter().map(|x| x.id).collect::<Vec<_>>(), vec![b.id]);
        assert!(list_open(&pool).await.unwrap().is_empty());
        assert!(!has_open(&pool, &task).await.unwrap());
        assert_eq!(get(&pool, b.id).await.unwrap().unwrap().status, CANCELLED);
    }

    #[tokio::test]
    async fn questions_of_a_finished_card_are_not_listed() {
        let (pool, task) = pool_with_running_task().await;
        create(&pool, &task, None, &q("A？", &[], None)).await.unwrap();
        crate::tasks::store::finish_task(&pool, &task, "success", None, None).await.unwrap();
        assert!(list_open(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn reminders_every_30_minutes_at_most_three_times() {
        let (pool, task) = pool_with_running_task().await;
        let a = create(&pool, &task, None, &q("A？", &[], None)).await.unwrap();
        let t0 = a.created_at;
        assert!(due_reminders(&pool, t0 + REMIND_EVERY_SECS - 1).await.unwrap().is_empty());
        let mut now = t0 + REMIND_EVERY_SECS;
        for _ in 0..MAX_REMINDERS {
            assert_eq!(due_reminders(&pool, now).await.unwrap().len(), 1);
            mark_reminded(&pool, a.id, now).await.unwrap();
            assert!(due_reminders(&pool, now + 60).await.unwrap().is_empty(), "剛提醒過");
            now += REMIND_EVERY_SECS;
        }
        assert!(due_reminders(&pool, now).await.unwrap().is_empty(), "最多提醒 {MAX_REMINDERS} 次");
    }

    #[tokio::test]
    async fn telegram_message_ids_map_back_to_their_question() {
        let (pool, task) = pool_with_running_task().await;
        let a = create(&pool, &task, None, &q("A？", &[], None)).await.unwrap();
        set_telegram_message(&pool, a.id, 777).await.unwrap();
        assert_eq!(by_telegram_message(&pool, 777).await.unwrap().map(|x| x.id), Some(a.id));
        assert_eq!(by_telegram_message(&pool, 778).await.unwrap(), None);
    }

    #[test]
    fn messages_to_claude_name_the_question_and_remind_about_the_marker() {
        let row = QuestionRow {
            id: 12, task_id: "t".into(), agent_id: None, question: "A？".into(), options: "[]".into(), recommended: None,
            context: None, status: ANSWERED.into(), answer: Some("中文".into()), answered_via: Some("app".into()),
            telegram_message_id: None, reminders: 0, last_reminded_at: None, created_at: 0, answered_at: Some(1),
        };
        let reply = ask_reply(&row);
        assert!(reply.contains("Q-12") && reply.contains("結束這一輪") && reply.contains("不要印出完成標記"), "{reply}");
        let msg = answer_message(&row, "（印出 <<X>>）");
        assert!(msg.starts_with("Q-12 的回答：中文"), "{msg}");
        assert!(msg.contains("（印出 <<X>>）"));
    }
}
